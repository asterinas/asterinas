// SPDX-License-Identifier: MPL-2.0

//! Useful synchronization primitives.

mod guard;
mod mutex;
mod rcu;
mod rwarc;
mod rwlock;
mod rwmutex;
mod spin;
mod wait;

pub(crate) use self::{
    guard::GuardTransfer,
    rcu::{after_grace_period, finish_grace_period},
};
pub use self::{
    guard::{LocalIrqDisabled, PreemptDisabled, WriteIrqDisabled},
    mutex::{Mutex, MutexGuard},
    rcu::{OwnerPtr, Rcu, RcuOption, RcuReadGuard},
    rwarc::{RoArc, RwArc},
    rwlock::{RwLock, RwLockReadGuard, RwLockUpgradeableGuard, RwLockWriteGuard},
    rwmutex::{RwMutex, RwMutexReadGuard, RwMutexUpgradeableGuard, RwMutexWriteGuard},
    spin::{SpinLock, SpinLockGuard},
    wait::{WaitQueue, Waiter, Waker},
};

pub(crate) fn init() {
    rcu::init();
}
