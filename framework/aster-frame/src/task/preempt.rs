// SPDX-License-Identifier: MPL-2.0

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
    /// The number of locks and irq-disabled contexts held by the current CPU.
    num: AtomicUsize,
    in_preemption: AtomicBool,
}

impl PreemptInfo {
    const fn new() -> Self {
        Self {
            num: AtomicUsize::new(0),
            in_preemption: AtomicBool::new(false),
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

    fn in_preemption(&self) -> bool {
        self.in_preemption.load(Relaxed)
    }

    fn activate(&self) {
        self.in_preemption.store(false, Relaxed);
        if !is_local_irq_enabled() {
            enable_local_irq();
        }
    }

    fn deactivate(&self) {
        if self.in_preemption.load(Relaxed) {
            panic!("Nested preemption is not allowed on in_preemption flag.");
        }
        if is_local_irq_enabled() {
            disable_local_irq();
        }
        self.in_preemption.store(true, Relaxed);
    }

    fn is_preemptible(&self) -> bool {
        // todo: may cause inconsistency
        !self.in_preemption() && !self.in_atomic()
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
    Irq(_Guard),
    Sched(_Guard),
}
impl !Send for DisablePreemptGuard {}

impl DisablePreemptGuard {
    pub fn for_irq() -> Self {
        PREEMPT_COUNT.inc_num();
        Self::Irq(_Guard { _private: () })
    }

    pub fn for_lock() -> Self {
        PREEMPT_COUNT.inc_num();
        Self::Lock(_Guard { _private: () })
    }

    /// Transfer this guard to a new guard.
    /// This guard must be dropped after this function.
    pub fn transfer_to(&self) -> Self {
        assert!(matches!(self, Self::Lock(_)));
        Self::for_lock()
    }
}

impl Drop for DisablePreemptGuard {
    fn drop(&mut self) {
        match self {
            Self::Irq(_) => {
                PREEMPT_COUNT.dec_num();
            }
            Self::Lock(_) => {
                PREEMPT_COUNT.dec_num();
            }
            Self::Sched(_) => {
                PREEMPT_COUNT.activate();
            }
        }
    }
}

/// Whether the current CPU is in atomic context,
/// which means it holds some locks with irq disabled or is in irq context.
pub fn in_atomic() -> bool {
    PREEMPT_COUNT.in_atomic()
}

/// Whether the current CPU is preemptible, which means it is
/// neither in atomic context, nor in IRQ context and the preemption is enabled.
/// If it is not preemptible, the CPU cannot call `schedule()`.
pub fn preemptible() -> bool {
    PREEMPT_COUNT.is_preemptible()
}

pub fn is_in_preemption() -> bool {
    PREEMPT_COUNT.in_preemption()
}

/// Allow preemption on the current CPU.
/// However, preemptible or not actually depends on the counter in `PREEMPT_COUNT`.
pub fn activate_preempt() {
    PREEMPT_COUNT.activate();
}

/// Disalbe all preemption on the current CPU.
pub fn deactivate_preempt() {
    PREEMPT_COUNT.deactivate();
}

// TODO: impl might_sleep
pub fn panic_if_in_atomic() {
    if !in_atomic() {
        return;
    }
    panic!(
        "The CPU is not atomic: preempt_count was {} with the in_preemption flag as {}.",
        PREEMPT_COUNT.num(),
        PREEMPT_COUNT.in_preemption()
    );
}
