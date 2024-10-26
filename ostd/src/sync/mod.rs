// SPDX-License-Identifier: MPL-2.0

//! Useful synchronization primitives.

mod guard;
mod mutex;
mod rcu;
mod rwarc;
mod rwlock;
mod rwmutex;
pub(crate) mod spin;
mod wait;

pub(crate) use self::rcu::finish_grace_period;
pub use self::{
    guard::{GuardTransfer, LocalIrqDisabled, PreemptDisabled, SpinGuardian, WriteIrqDisabled},
    mutex::{Mutex, MutexGuard},
    rcu::{non_null, Rcu, RcuDrop, RcuOption, RcuOptionReadGuard, RcuReadGuard},
    rwarc::{RoArc, RwArc},
    rwlock::{RwLock, RwLockReadGuard, RwLockUpgradeableGuard, RwLockWriteGuard},
    rwmutex::{RwMutex, RwMutexReadGuard, RwMutexUpgradeableGuard, RwMutexWriteGuard},
    spin::{SpinLock, SpinLockGuard},
    wait::{WaitQueue, Waiter, Waker},
};

pub(crate) fn init() {
    rcu::init();
}
