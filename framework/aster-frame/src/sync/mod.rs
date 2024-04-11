// SPDX-License-Identifier: MPL-2.0

mod atomic_bits;
mod mutex;
// TODO: refactor this rcu implementation
// Comment out this module since it raises lint error
mod rcu;
mod rwlock;
mod rwmutex;
mod spin;
mod wait;

pub use self::{
    atomic_bits::AtomicBits,
    mutex::{Mutex, MutexGuard},
    rcu::{pass_quiescent_state, OwnerPtr, Rcu, RcuReadGuard, RcuReclaimer},
    rwlock::{RwLock, RwLockReadGuard, RwLockUpgradeableGuard, RwLockWriteGuard},
    rwmutex::{RwMutex, RwMutexReadGuard, RwMutexUpgradeableGuard, RwMutexWriteGuard},
    spin::{SpinLock, SpinLockGuard},
    wait::WaitQueue,
};

pub(crate) fn init() {
    rcu::init();
}
