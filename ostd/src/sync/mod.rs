// SPDX-License-Identifier: MPL-2.0

//! Useful synchronization primitives.

mod guard;
mod mutex;
// TODO: refactor this rcu implementation
// Comment out this module since it raises lint error
// mod rcu;
mod rwarc;
mod rwlock;
mod rwmutex;
mod spin;
mod wait;

// pub use self::rcu::{pass_quiescent_state, OwnerPtr, Rcu, RcuReadGuard, RcuReclaimer};
pub(crate) use self::guard::GuardTransfer;
pub use self::{
    guard::{LocalIrqDisabled, PreemptDisabled, WriteIrqDisabled},
    mutex::{Mutex, MutexGuard},
    rwarc::{RoArc, RwArc},
    rwlock::{RwLock, RwLockReadGuard, RwLockUpgradeableGuard, RwLockWriteGuard},
    rwmutex::{RwMutex, RwMutexReadGuard, RwMutexUpgradeableGuard, RwMutexWriteGuard},
    spin::{SpinLock, SpinLockGuard},
    wait::{WaitQueue, Waiter, Waker},
};
