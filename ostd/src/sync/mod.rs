// SPDX-License-Identifier: MPL-2.0

//! Useful synchronization primitives.

mod guard;
mod mutex;
// TODO: refactor this rcu implementation
// Comment out this module since it raises lint error
// mod rcu;
mod rwlock;
mod rwmutex;
mod spin;
mod wait;

// pub use self::rcu::{pass_quiescent_state, OwnerPtr, Rcu, RcuReadGuard, RcuReclaimer};
pub(crate) use self::guard::GuardTransfer;
pub use self::{
    guard::{LocalIrqDisabled, PreemptDisabled, WriteIrqDisabled},
    mutex::{ArcMutexGuard, Mutex, MutexGuard},
    rwlock::{
        ArcRwLockReadGuard, ArcRwLockUpgradeableGuard, ArcRwLockWriteGuard, RwLock,
        RwLockReadGuard, RwLockUpgradeableGuard, RwLockWriteGuard,
    },
    rwmutex::{
        ArcRwMutexReadGuard, ArcRwMutexUpgradeableGuard, ArcRwMutexWriteGuard, RwMutex,
        RwMutexReadGuard, RwMutexUpgradeableGuard, RwMutexWriteGuard,
    },
    spin::{ArcSpinLockGuard, SpinLock, SpinLockGuard},
    wait::{WaitQueue, Waiter, Waker},
};
