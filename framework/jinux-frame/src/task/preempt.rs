use crate::arch::irq::is_local_enabled;
use crate::{cpu::CpuLocal, cpu_local};
use core::sync::atomic::Ordering::Relaxed;
use core::sync::atomic::{AtomicBool, AtomicUsize};

cpu_local! {
    static PREEMPT_COUNT: PreemptInfo = PreemptInfo::new();
}

/// When it has a non-zero value, the CPU cannot call ``schedule()``.
/// todo: unify the fields to avoid the inconsistency.
#[derive(Debug)]
struct PreemptInfo {
    num_locks: AtomicUsize,
    num_soft_irq: AtomicUsize,
    num_hard_irq: AtomicUsize,
    active: AtomicBool,
}

impl PreemptInfo {
    const fn new() -> Self {
        Self {
            num_locks: AtomicUsize::new(0),
            active: AtomicBool::new(false),
            num_soft_irq: AtomicUsize::new(0),
            num_hard_irq: AtomicUsize::new(0),
        }
    }

    /// The locks and IRQs held by the current CPU.
    /// Return the number of locks, soft IRQs, hard IRQs and the active flag.
    fn stat(&self) -> (usize, usize, usize, bool) {
        // todo: into an atomic implementation
        (
            self.num_locks(),
            self.num_soft_irq(),
            self.num_hard_irq(),
            self.is_active(),
        )
    }

    fn num_locks(&self) -> usize {
        self.num_locks.load(Relaxed)
    }

    fn inc_num_locks(&self) -> usize {
        self.num_locks.fetch_add(1, Relaxed)
    }

    fn dec_num_locks(&self) -> usize {
        self.num_locks.fetch_sub(1, Relaxed)
    }

    fn num_hard_irq(&self) -> usize {
        self.num_hard_irq.load(Relaxed)
    }

    fn inc_num_hard_irq(&self) -> usize {
        self.num_hard_irq.fetch_add(1, Relaxed)
    }

    fn dec_num_hard_irq(&self) -> usize {
        self.num_hard_irq.fetch_sub(1, Relaxed)
    }

    fn num_soft_irq(&self) -> usize {
        self.num_soft_irq.load(Relaxed)
    }

    fn inc_num_soft_irq(&self) -> usize {
        self.num_soft_irq.fetch_add(1, Relaxed)
    }

    fn dec_num_soft_irq(&self) -> usize {
        self.num_soft_irq.fetch_sub(1, Relaxed)
    }

    fn is_active(&self) -> bool {
        self.active.load(Relaxed)
    }

    fn activate(&self) {
        self.active.store(true, Relaxed);
    }

    fn deactivate(&self) {
        self.active.store(false, Relaxed);
    }

    fn is_preempted(&self) -> bool {
        // todo: may cause inconsistency
        self.is_active() || self.in_atomic()
    }

    /// Whether the current CPU is in atomic context,
    /// which means it holds some locks or is in IRQ context.
    fn in_atomic(&self) -> bool {
        self.num_locks() != 0 || self.in_irq()
    }

    /// Whether the current CPU is in IRQ context.
    fn in_irq(&self) -> bool {
        self.num_soft_irq() != 0 || self.num_hard_irq() != 0
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
pub enum DisablePreemptGuard {
    Lock(_Guard),
    HardIrq(_Guard),
    SoftIrq(_Guard),
}
impl !Send for DisablePreemptGuard {}

impl DisablePreemptGuard {
    pub fn lock() -> Self {
        PREEMPT_COUNT.inc_num_locks();
        Self::Lock(_Guard { _private: () })
    }

    pub fn hard_irq() -> Self {
        PREEMPT_COUNT.inc_num_hard_irq();
        Self::HardIrq(_Guard { _private: () })
    }

    pub fn soft_irq() -> Self {
        PREEMPT_COUNT.inc_num_soft_irq();
        Self::SoftIrq(_Guard { _private: () })
    }

    /// Transfer this guard to a new guard.
    /// This guard must be dropped after this function.
    pub fn transfer_to(&self) -> Self {
        match self {
            Self::Lock(_) => Self::lock(),
            Self::HardIrq(_) => Self::hard_irq(),
            Self::SoftIrq(_) => Self::soft_irq(),
        }
    }
}

impl Drop for DisablePreemptGuard {
    fn drop(&mut self) {
        match self {
            Self::Lock(_) => {
                PREEMPT_COUNT.dec_num_locks();
            }
            Self::HardIrq(_) => {
                PREEMPT_COUNT.dec_num_hard_irq();
            }
            Self::SoftIrq(_) => {
                PREEMPT_COUNT.dec_num_soft_irq();
            }
        }
    }
}

/// Whether the current CPU is in atomic context,
/// which means it holds some locks or is in IRQ context.
pub fn in_atomic() -> bool {
    PREEMPT_COUNT.in_atomic()
}

/// Whether the current CPU is in IRQ context.
pub fn in_irq() -> bool {
    PREEMPT_COUNT.in_irq()
}

/// Whether the current CPU is preemptible, which means it is
/// neither in atomic context, nor in IRQ context and the preemption is enabled.
pub fn preemptible() -> bool {
    !PREEMPT_COUNT.is_preempted() && is_local_enabled()
}

/// The locks and IRQs held by the current CPU.
/// Return the number of locks, soft IRQs, hard IRQs and the active flag.
pub fn preempt_stat() -> (usize, usize, usize, bool) {
    PREEMPT_COUNT.stat()
}
