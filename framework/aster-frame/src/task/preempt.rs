use crate::arch::irq::{
    disable_local as disable_local_irq, enable_local as enable_local_irq,
    is_local_enabled as is_local_irq_enabled,
};
use crate::{cpu::CpuLocal, cpu_local};
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::{AtomicBool, AtomicUsize};

cpu_local! {
    static PREEMPT_COUNT: PreemptInfo = PreemptInfo::new();
}

// todo: unify the fields to avoid the inconsistency.
#[derive(Debug)]
struct PreemptInfo {
    num: AtomicUsize,
    /// If available for preemption.
    active: AtomicBool,
}

impl PreemptInfo {
    const fn new() -> Self {
        Self {
            num: AtomicUsize::new(0),
            active: AtomicBool::new(true),
        }
    }

    fn num(&self) -> usize {
        self.num.load(Relaxed)
    }

    fn inc_num(&self) -> usize {
        self.num.fetch_add(1, Relaxed)
    }

    fn dec_num(&self) -> usize {
        self.num.fetch_sub(1, Relaxed)
    }

    fn is_active(&self) -> bool {
        self.active.load(Relaxed)
    }

    fn activate(&self) {
        self.active.store(true, Relaxed);
        if !is_local_irq_enabled() {
            enable_local_irq();
        }
    }

    fn deactivate(&self) {
        if !self.active.load(Relaxed) {
            panic!("Nested preemption is not allowed on active flag.");
        }
        if is_local_irq_enabled() {
            disable_local_irq();
        }
        self.active.store(false, Relaxed);
    }

    fn is_preempted(&self) -> bool {
        // todo: may cause inconsistency
        !self.is_active() || self.in_atomic()
    }

    /// Whether the current CPU is in atomic context,
    /// which means it holds some locks or is in IRQ context.
    fn in_atomic(&self) -> bool {
        self.num() != 0
    }
}

/// A private type to prevent user from constructing DisablePreemptGuard directly.
struct _Guard {
    /// This private field prevents user from constructing values of this type directly.
    _private: (),
}
impl !Send for _Guard {}

/// A guard to disable preempt.
#[allow(private_interfaces)]
#[clippy::has_significant_drop]
pub enum DisablePreemptGuard {
    Lock(_Guard),
    Kernel(_Guard),
}
impl !Send for DisablePreemptGuard {}

impl Default for DisablePreemptGuard {
    fn default() -> Self {
        Self::new()
    }
}

impl DisablePreemptGuard {
    pub fn new() -> Self {
        PREEMPT_COUNT.inc_num();
        Self::Lock(_Guard { _private: () })
    }

    pub fn kernel() -> Self {
        PREEMPT_COUNT.activate();
        Self::Kernel(_Guard { _private: () })
    }

    /// Transfer this guard to a new guard.
    /// This guard must be dropped after this function.
    pub fn transfer_to(&self) -> Self {
        match self {
            Self::Lock(_) => Self::new(),
            Self::Kernel(_) => Self::kernel(),
        }
    }
}

impl Drop for DisablePreemptGuard {
    fn drop(&mut self) {
        match self {
            Self::Lock(_) => {
                PREEMPT_COUNT.dec_num();
            }
            Self::Kernel(_) => {
                PREEMPT_COUNT.activate();
            }
        }
    }
}

/// Whether the current CPU is in atomic context,
/// which means it holds some locks or is in IRQ context.
pub fn in_atomic() -> bool {
    PREEMPT_COUNT.in_atomic()
}

/// Whether the current CPU is preemptible, which means it is
/// neither in atomic context, nor in IRQ context and the preemption is enabled.
/// If it is not preemptible, the CPU cannot call `schedule()`.
pub fn preemptible() -> bool {
    !PREEMPT_COUNT.is_preempted()
}

pub fn is_preempt_count_active() -> bool {
    PREEMPT_COUNT.is_active()
}

pub fn activate_preempt() {
    PREEMPT_COUNT.activate();
}

pub fn deactivate_preempt() {
    PREEMPT_COUNT.deactivate();
}

// TODO: impl might_sleep
pub fn panic_if_in_atomic() {
    if !in_atomic() {
        return;
    }
    panic!(
        "The CPU is not atomic: preempt_count was {} with the active flag as {}.",
        PREEMPT_COUNT.num(),
        PREEMPT_COUNT.is_active()
    );
}
