// SPDX-License-Identifier: MPL-2.0

//! Useful synchronization primitives.

mod mutex;
mod rcu;
mod rwlock;
mod rwmutex;
mod spin;
mod wait;

pub use self::{
    mutex::{ArcMutexGuard, Mutex, MutexGuard},
    rcu::{pass_quiescent_state, LazyRcu, OwnerPtr, Rcu, RcuReadGuard},
    rwlock::{
        ArcRwLockReadGuard, ArcRwLockUpgradeableGuard, ArcRwLockWriteGuard, RwLock,
        RwLockReadGuard, RwLockUpgradeableGuard, RwLockWriteGuard,
    },
    rwmutex::{
        ArcRwMutexReadGuard, ArcRwMutexUpgradeableGuard, ArcRwMutexWriteGuard, RwMutex,
        RwMutexReadGuard, RwMutexUpgradeableGuard, RwMutexWriteGuard,
    },
    spin::{ArcSpinLockGuard, LocalIrqDisabled, PreemptDisabled, SpinLock, SpinLockGuard},
    wait::{WaitQueue, Waiter, Waker},
};

pub(crate) fn init() {
    rcu::init();
}
