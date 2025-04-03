// SPDX-License-Identifier: MPL-2.0

use ostd::{
    sync::{GuardTransfer, SpinGuardian},
    task::{
        atomic_mode::{AsAtomicModeGuard, InAtomicMode},
        disable_preempt, DisabledPreemptGuard,
    },
};

use crate::{disable_softirq_local, DisableLocalSoftirqGuard};

/// A guardian that disables bottom half while holding a lock.
pub enum BottomHalfDisabled {}

pub struct DisableLocalBottomHalfGuard {
    _preempt: DisabledPreemptGuard,
    _softirq: DisableLocalSoftirqGuard,
}

impl AsAtomicModeGuard for DisableLocalBottomHalfGuard {
    fn as_atomic_mode_guard(&self) -> &dyn InAtomicMode {
        &self._preempt
    }
}

#[must_use]
fn disable_local_bottom_half() -> DisableLocalBottomHalfGuard {
    // When disabling softirq, we must also disable preemption
    // to avoid the task to be scheduled to other CPUs.
    let _preempt = disable_preempt();
    let _softirq = disable_softirq_local();
    DisableLocalBottomHalfGuard { _preempt, _softirq }
}

impl GuardTransfer for DisableLocalBottomHalfGuard {
    fn transfer_to(&mut self) -> Self {
        Self {
            _preempt: disable_preempt(),
            _softirq: disable_softirq_local(),
        }
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
    use crate::is_softirq_enabled;

    #[ktest]
    fn test_spinlock() {
        let lock = SpinLock::<(), BottomHalfDisabled>::new(());

        let softirq_enabled = is_softirq_enabled();

        let guard = lock.lock();
        assert!(!is_softirq_enabled());

        drop(guard);
        assert_eq!(is_softirq_enabled(), softirq_enabled);
    }

    #[ktest]
    fn test_nested_spin_lock() {
        let lock1 = SpinLock::<(), BottomHalfDisabled>::new(());
        let lock2 = SpinLock::<(), BottomHalfDisabled>::new(());

        let softirq_enabled = is_softirq_enabled();

        let guard1 = lock1.lock();
        let guard2 = lock2.lock();
        assert!(!is_softirq_enabled());

        drop(guard1);
        assert!(!is_softirq_enabled());

        drop(guard2);
        assert_eq!(is_softirq_enabled(), softirq_enabled);
    }

    #[ktest]
    fn test_rwlock() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        let softirq_enabled = is_softirq_enabled();

        let write_guard = rwlock.write();
        assert!(!is_softirq_enabled());

        drop(write_guard);
        assert_eq!(is_softirq_enabled(), softirq_enabled);
    }

    #[ktest]
    fn test_nested_rwlock() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        let softirq_enabled = is_softirq_enabled();

        let read_guard1 = rwlock.read();
        let read_guard2 = rwlock.read();
        assert!(!is_softirq_enabled());

        drop(read_guard1);
        assert!(!is_softirq_enabled());

        drop(read_guard2);
        assert_eq!(is_softirq_enabled(), softirq_enabled);
    }

    #[test]
    fn test_rwlock_upgrade() {
        let rwlock: RwLock<(), BottomHalfDisabled> = RwLock::new(());

        let softirq_enabled = is_softirq_enabled();

        let upgrade_guard = rwlock.upread();
        assert!(!is_softirq_enabled());

        let write_guard = upgrade_guard.upgrade();
        assert!(!is_softirq_enabled());

        let read_guard = write_guard.downgrade();
        assert!(!is_softirq_enabled());

        drop(read_guard);
        assert_eq!(is_softirq_enabled(), softirq_enabled);
    }
}
