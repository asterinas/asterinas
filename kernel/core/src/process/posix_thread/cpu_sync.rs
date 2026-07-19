// SPDX-License-Identifier: MPL-2.0

use core::cell::{Cell, RefCell};

use ostd::irq::DisabledLocalIrqGuard;

/// A smart cache of one user-mode CPU register
/// to minimize the expensive save/restore traffic
/// between memory and the CPU's hardware register.
///
/// # How it works
///
/// ## Canonical values
///
/// To minimize the expensive saves/restores from/to the CPU hardware registers,
/// `CpuSync<R>` tracks *where* the latest **canonical value** of
/// a register of type `R` currently lives
/// and uses that to elide redundant work.
/// At every moment in time, the canonical value lives
/// in one or both of two places,
/// tracked by [`CanonicalValueLocation`]:
///
/// | Location   | Memory canonical | CPU canonical |
/// | ---------- | :--------------: | :-----------: |
/// | `InMemory` | ✓                | ✗             |
/// | `OnCpu`    | ✗                | ✓             |
/// | `Both`     | ✓                | ✓             |
///
/// ## Invariant
///
/// `CpuSync<R>` preserves the invariant that
/// **the recorded location reflects where the canonical value actually resides.**
/// Four methods cooperate to maintain it:
///
/// - The [`before_schedule`] event hook method runs
///   just before switching out the current task.
/// - The [`before_user_exec`] event hook method runs
///   just before entering the user space.
/// - The [`get`] accessor method returns the canonical value.
/// - The [`set`] accessor method writes a new canonical value into memory.
///
/// The two event hooks are deliberately placed at *the latest moments*
/// where save and restore are still correct.
/// Delaying as far as possible allows `CpuSync`
/// to do as many optimizations as possible
///
/// ## Optimizations
///
/// Thanks to the invariant,
/// `CpuSync<R>` takes the optimization opportunities below:
///
/// - At [`before_schedule`]:
///   an expensive `<R as UserReg>::save_from_cpu` is needed
///   *only if* the canonical value is `OnCpu`;
///   if it's `InMemory` or `Both`, then the save is skipped safely.
///   Either way the location transitions to `InMemory`
///   because the imminent context switch clobbers the CPU.
/// - At [`before_user_exec`]:
///   an expensive `<R as UserReg>::restore_to_cpu` is needed
///   *only if* the value is `InMemory`;
///   if it's `OnCpu` (successive loop iterations) or
///   `Both` (a prior `get` followed by no schedule),
///   then the load is skipped safely.
/// - At [`get`]:
///   an expensive `<R as UserReg>::save_from_cpu` is needed
///   *only if* the value is `OnCpu`;
///   afterwards the location transitions to `Both`,
///   so a subsequent `before_schedule` can skip the save.
/// - At [`set`]:
///   nothing touches the CPU; the value is written to memory and
///   the location transitions to `InMemory`, so the new value reaches the CPU
///   lazily at the next `before_user_exec` (batching multiple sets into one
///   load, and performing no CPU write at all if the task exits first).
///
/// # Concurrency
///
/// `CpuSync<R>` is intended to live inside a per-thread `ThreadLocal`
/// and uses `Cell`/`RefCell` for interior mutability accordingly.
/// It is **not** `Sync` and must not be accessed across threads.
///
/// All methods assume kernel preemption cannot occur
/// for the duration of the call.
/// In the current architecture, this is always true
/// because kernel preemption was never implemented.
///
/// More importantly, we cannot implement kernel preemption
/// without refactoring the `ThreadLocal` mechanism
/// because `ThreadLocal` cannot be accessed in interrupt handlers
/// for soundness reasons.
/// But such access is necessary for the preempted schedule.
///
/// Therefore, we omit the preemption guards for better performance and
/// defer preemption considerations to future work.
///
/// [`before_schedule`]: Self::before_schedule
/// [`before_user_exec`]: Self::before_user_exec
/// [`get`]: Self::get
/// [`set`]: Self::set
#[derive(Debug)]
pub struct CpuSync<R> {
    location: Cell<CanonicalValueLocation>,
    reg: RefCell<R>,
}

impl<R: UserReg> CpuSync<R> {
    /// Constructs a `CpuSync` holding `reg`,
    /// with the canonical value initially `InMemory`.
    ///
    /// The CPU register is presumed to
    /// hold some other task's bytes (or nobody's)
    /// until the first [`Self::before_user_exec`] loads `reg` onto it.
    pub(super) const fn new(reg: R) -> Self {
        Self {
            location: Cell::new(CanonicalValueLocation::InMemory),
            reg: RefCell::new(reg),
        }
    }

    /// Gets a copy of the current canonical value.
    ///
    /// This method guarantees that
    /// the returned value reflects the very latest state.
    pub fn get(&self) -> R
    where
        R: Clone,
    {
        if self.location.get() == CanonicalValueLocation::OnCpu {
            self.reg.borrow_mut().save_from_cpu();
            self.location.set(CanonicalValueLocation::Both);
        }
        self.reg.borrow().clone()
    }

    /// Sets the canonical value.
    ///
    /// The new value reaches the CPU on the next [`before_user_exec`].
    /// The `set` method itself only writes the in-memory copy and marks `InMemory`.
    /// This avoids a CPU write when:
    ///
    /// 1. `set` is followed by more `set`s before user-entry —
    ///    they collapse into a single load.
    /// 2. The task never re-enters user mode
    ///    after this `set` (e.g. exits in the kernel) — no CPU write happens at all.
    /// 3. A context switch intervenes — the new value rides into the CPU on
    ///    the eventual `before_user_exec`, with `before_schedule` a no-op
    ///    along the way (since the location is already `InMemory`).
    ///
    /// [`before_user_exec`]: Self::before_user_exec
    pub fn set(&self, new_reg: R) {
        *self.reg.borrow_mut() = new_reg;
        if self.location.get() != CanonicalValueLocation::InMemory {
            self.location.set(CanonicalValueLocation::InMemory);
        }
    }

    // ---- Lifecycle callbacks ----

    /// Scheduler hook: this task is about to be switched off the CPU.
    ///
    /// This hook method ensures that
    /// the canonical value of the register `R` is saved into `self`
    /// and the canonical value location is marked `InMemory`.
    pub(super) fn before_schedule(&self, guard: &DisabledLocalIrqGuard) {
        let loc = self.location.get();
        if loc == CanonicalValueLocation::OnCpu {
            self.reg.borrow_mut().save_from_cpu_with_irq_disabled(guard);
        }
        if loc != CanonicalValueLocation::InMemory {
            self.location.set(CanonicalValueLocation::InMemory);
        }
    }

    /// User-task-loop hook: this task is about to enter user mode.
    ///
    /// Must be called immediately before every userspace entry,
    /// before any user-visible CPU instruction runs.
    ///
    /// This hook method ensures that
    /// the canonical value of the register `R` is loaded to the CPU hardware
    /// and the canonical value location is marked `OnCpu`.
    pub(super) fn before_user_exec(&self, guard: &DisabledLocalIrqGuard) {
        if self.location.get() == CanonicalValueLocation::InMemory {
            self.reg.borrow().restore_to_cpu_with_irq_disabled(guard);
        }
        self.location.set(CanonicalValueLocation::OnCpu);
    }
}

/// A user-mode CPU register
/// whose in-memory value may be moved to and from the CPU's hardware register.
///
/// Implementors typically wrap one or a small bundle of registers/MSRs
/// (e.g. the user FS base, the user GS base, or the FPU XSAVE area)
/// into a struct that implements this trait.
pub trait UserReg {
    /// Saves the current value from the CPU register(s) into `self`.
    ///
    /// May be invoked in any IRQ state.
    fn save_from_cpu(&mut self);

    /// Saves the current value from the CPU register(s) into `self`,
    /// with the caller's guarantee that local IRQs are disabled.
    ///
    /// Implementors may override to take a cheaper path
    /// that is only safe with IRQs disabled
    /// The default falls back to [`Self::save_from_cpu`].
    fn save_from_cpu_with_irq_disabled(&mut self, _guard: &DisabledLocalIrqGuard) {
        self.save_from_cpu();
    }

    /// Writes `self`'s value into the CPU register(s).
    ///
    /// May be invoked in any IRQ state.
    fn restore_to_cpu(&self);

    /// Writes `self`'s value into the CPU register(s),
    /// with the caller's guarantee that local IRQs are disabled.
    ///
    /// Implementors may override to take a cheaper path
    /// that is only safe with IRQs disabled
    /// The default falls back to [`Self::restore_to_cpu`].
    fn restore_to_cpu_with_irq_disabled(&self, _guard: &DisabledLocalIrqGuard) {
        self.restore_to_cpu();
    }
}

/// Where the latest canonical value of a [`UserReg`] currently lives.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum CanonicalValueLocation {
    /// Only the in-memory copy holds the canonical value.
    InMemory,
    /// Only the CPU register holds the canonical value.
    OnCpu,
    /// Both the in-memory copy and the CPU register hold the canonical value.
    Both,
}

// ---------- `UserReg` impls for OSTD register types ----------

impl UserReg for ostd::arch::cpu::context::FpuContext {
    fn save_from_cpu(&mut self) {
        self.save();
    }

    fn restore_to_cpu(&self) {
        self.load();
    }
}

#[cfg(target_arch = "x86_64")]
impl UserReg for ostd::arch::cpu::context::FsBase {
    fn save_from_cpu(&mut self) {
        self.save();
    }

    fn restore_to_cpu(&self) {
        self.load();
    }
}

#[cfg(target_arch = "x86_64")]
impl UserReg for ostd::arch::cpu::context::GsBase {
    fn save_from_cpu(&mut self) {
        let guard = ostd::irq::disable_local();
        self.save(&guard);
    }

    fn save_from_cpu_with_irq_disabled(&mut self, guard: &DisabledLocalIrqGuard) {
        self.save(guard);
    }

    fn restore_to_cpu(&self) {
        let guard = ostd::irq::disable_local();
        self.load(&guard);
    }

    fn restore_to_cpu_with_irq_disabled(&self, guard: &DisabledLocalIrqGuard) {
        self.load(guard)
    }
}
