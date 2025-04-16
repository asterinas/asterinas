// SPDX-License-Identifier: MPL-2.0

use ostd::{
    cpu_local_cell,
    sync::{GuardTransfer, SpinGuardian},
    task::{
        atomic_mode::{AsAtomicModeGuard, InAtomicMode},
        disable_preempt, DisabledPreemptGuard,
    },
    trap::{in_interrupt_context, irq::disable_local},
};

use crate::process_all_pending;

cpu_local_cell! {
    static DISABLE_SOFTIRQ_COUNT: u8 = 0;
}

/// Returns whether softirq is enabled on local CPU.
pub(super) fn is_softirq_enabled() -> bool {
    DISABLE_SOFTIRQ_COUNT.load() == 0
}

/// A guardian that disables bottom half while holding a lock.
pub enum BottomHalfDisabled {}

/// A guard for disabled local softirqs.
pub struct DisableLocalBottomHalfGuard {
    preempt: DisabledPreemptGuard,
}

impl AsAtomicModeGuard for DisableLocalBottomHalfGuard {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        &self.preempt
    }
}

impl Drop for DisableLocalBottomHalfGuard {
    fn drop(&mut self) {
        // Once the guard is dropped, we will process pending items within
        // the current thread's context if softirq is going to be enabled.
        // This behavior is similar to how Linux handles pending softirqs.
        if DISABLE_SOFTIRQ_COUNT.load() == 1 && !in_interrupt_context() {
            // Preemption and softirq are not really enabled at the moment,
            // so we can guarantee that we'll process any pending softirqs for the current CPU.
            let irq_guard = disable_local();
            let irq_guard = process_all_pending(irq_guard);

            // To avoid race conditions, we should decrease the softirq count first,
            // then drop the IRQ guard.
            DISABLE_SOFTIRQ_COUNT.sub_assign(1);
            drop(irq_guard);
            return;
        }

        DISABLE_SOFTIRQ_COUNT.sub_assign(1);
    }
}

#[must_use]
fn disable_local_bottom_half() -> DisableLocalBottomHalfGuard {
    // When disabling softirq, we must also disable preemption
    // to avoid the task to be scheduled to other CPUs.
    let preempt = disable_preempt();
    DISABLE_SOFTIRQ_COUNT.add_assign(1);
    DisableLocalBottomHalfGuard { preempt }
}

impl GuardTransfer for DisableLocalBottomHalfGuard {
    fn transfer_to(&mut self) -> Self {
        disable_local_bottom_half()
    }
}

impl SpinGuardian for BottomHalfDisabled {
    type Guard = DisableLocalBottomHalfGuard;
    type ReadGuard = DisableLocalBottomHalfGuard;

    fn read_guard() -> Self::ReadGuard {
        disable_local_bottom_half()
    }

    fn guard() -> Self::Guard {
        disable_local_bottom_half()
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{
        prelude::*,
        sync::{RwLock, SpinLock},
    };

    use super::*;

    #[ktest]
    fn spinlock_disable_bh() {
        let lock = SpinLock::<(), BottomHalfDisabled>::new(());

        assert!(is_softirq_enabled());

        let guard = lock.lock();
        assert!(!is_softirq_enabled());

        drop(guard);
        assert!(is_softirq_enabled());
    }

    #[ktest]
    fn nested_spin_lock_disable_bh() {
        let lock1 = SpinLock::<(), BottomHalfDisabled>::new(());
        let lock2 = SpinLock::<(), BottomHalfDisabled>::new(());

        assert!(is_softirq_enabled());

        let guard1 = lock1.lock();
        let guard2 = lock2.lock();
        assert!(!is_softirq_enabled());

        drop(guard1);
        assert!(!is_softirq_enabled());

        drop(guard2);
        assert!(is_softirq_enabled());
    }

    #[ktest]
    fn rwlock_disable_bh() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        assert!(is_softirq_enabled());

        let write_guard = rwlock.write();
        assert!(!is_softirq_enabled());

        drop(write_guard);
        assert!(is_softirq_enabled());
    }

    #[ktest]
    fn nested_rwlock_disable_bh() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        assert!(is_softirq_enabled());

        let read_guard1 = rwlock.read();
        let read_guard2 = rwlock.read();
        assert!(!is_softirq_enabled());

        drop(read_guard1);
        assert!(!is_softirq_enabled());

        drop(read_guard2);
        assert!(is_softirq_enabled());
    }

    #[test]
    fn upgradable_rwlock_disable_bh() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        assert!(is_softirq_enabled());

        let upgrade_guard = rwlock.upread();
        assert!(!is_softirq_enabled());

        let write_guard = upgrade_guard.upgrade();
        assert!(!is_softirq_enabled());

        let read_guard = write_guard.downgrade();
        assert!(!is_softirq_enabled());

        drop(read_guard);
        assert!(is_softirq_enabled());
    }
}
