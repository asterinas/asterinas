// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use alloc::sync::Arc;
use core::{
    cell::UnsafeCell,
    fmt,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
    task::{disable_preempt, DisablePreemptGuard},
    trap::{disable_local, DisabledLocalIrqGuard},
};

/// A spin lock.
pub struct SpinLock<T: ?Sized> {
    lock: AtomicBool,
    val: UnsafeCell<T>,
}

impl<T> SpinLock<T> {
    /// Creates a new spin lock.
    pub const fn new(val: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            val: UnsafeCell::new(val),
        }
    }
}

impl<T: ?Sized> SpinLock<T> {
    /// Acquires the spin lock with disabling the local IRQs. This is the most secure
    /// locking way.
    ///
    /// This method runs in a busy loop until the lock can be acquired.
    /// After acquiring the spin lock, all interrupts are disabled.
    pub fn lock_irq_disabled(&self) -> SpinLockGuard<T> {
        let guard = disable_local();
        self.acquire_lock();
        SpinLockGuard_ {
            lock: self,
            inner_guard: InnerGuard::IrqGuard(guard),
        }
    }

    /// Tries acquiring the spin lock immedidately with disabling the local IRQs.
    pub fn try_lock_irq_disabled(&self) -> Option<SpinLockGuard<T>> {
        let irq_guard = disable_local();
        if self.try_acquire_lock() {
            let lock_guard = SpinLockGuard_ {
                lock: self,
                inner_guard: InnerGuard::IrqGuard(irq_guard),
            };
            return Some(lock_guard);
        }
        None
    }

    /// Acquires the spin lock without disabling local IRQs.
    ///
    /// This method is twice as fast as the [`lock_irq_disabled`] method.
    /// So prefer using this method over the [`lock_irq_disabled`] method
    /// when IRQ handlers are allowed to get executed while
    /// holding this lock. For example, if a lock is never used
    /// in the interrupt context, then it is ok to use this method
    /// in the process context.
    ///
    /// [`lock_irq_disabled`]: Self::lock_irq_disabled
    pub fn lock(&self) -> SpinLockGuard<T> {
        let guard = disable_preempt();
        self.acquire_lock();
        SpinLockGuard_ {
            lock: self,
            inner_guard: InnerGuard::PreemptGuard(guard),
        }
    }

    /// Acquires the spin lock through an [`Arc`].
    ///
    /// The method is similar to [`lock`], but it doesn't have the requirement
    /// for compile-time checked lifetimes of the lock guard.
    ///
    /// [`lock`]: Self::lock
    pub fn lock_arc(self: &Arc<Self>) -> ArcSpinLockGuard<T> {
        let guard = disable_preempt();
        self.acquire_lock();
        SpinLockGuard_ {
            lock: self.clone(),
            inner_guard: InnerGuard::PreemptGuard(guard),
        }
    }

    /// Tries acquiring the spin lock immedidately without disabling the local IRQs.
    pub fn try_lock(&self) -> Option<SpinLockGuard<T>> {
        let guard = disable_preempt();
        if self.try_acquire_lock() {
            let lock_guard = SpinLockGuard_ {
                lock: self,
                inner_guard: InnerGuard::PreemptGuard(guard),
            };
            return Some(lock_guard);
        }
        None
    }

    /// Acquires the spin lock, otherwise busy waiting
    fn acquire_lock(&self) {
        while !self.try_acquire_lock() {
            core::hint::spin_loop();
        }
    }

    fn try_acquire_lock(&self) -> bool {
        self.lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn release_lock(&self) {
        self.lock.store(false, Ordering::Release);
    }
}

impl<T: ?Sized + fmt::Debug> fmt::Debug for SpinLock<T> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

// SAFETY: Only a single lock holder is permitted to access the inner data of Spinlock.
unsafe impl<T: ?Sized + Send> Send for SpinLock<T> {}
unsafe impl<T: ?Sized + Send> Sync for SpinLock<T> {}

enum InnerGuard {
    IrqGuard(DisabledLocalIrqGuard),
    PreemptGuard(DisablePreemptGuard),
}

/// A guard that provides exclusive access to the data protected by a [`SpinLock`].
pub type SpinLockGuard<'a, T> = SpinLockGuard_<T, &'a SpinLock<T>>;
/// A guard that provides exclusive access to the data protected by a `Arc<SpinLock>`.
pub type ArcSpinLockGuard<T> = SpinLockGuard_<T, Arc<SpinLock<T>>>;

/// The guard of a spin lock that disables the local IRQs.
pub struct SpinLockGuard_<T: ?Sized, R: Deref<Target = SpinLock<T>>> {
    inner_guard: InnerGuard,
    lock: R,
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T>>> Deref for SpinLockGuard_<T, R> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T>>> DerefMut for SpinLockGuard_<T, R> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T>>> Drop for SpinLockGuard_<T, R> {
    fn drop(&mut self) {
        self.lock.release_lock();
    }
}

impl<T: ?Sized + fmt::Debug, R: Deref<Target = SpinLock<T>>> fmt::Debug for SpinLockGuard_<T, R> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T>>> !Send for SpinLockGuard_<T, R> {}

// SAFETY: `SpinLockGuard_` can be shared between tasks/threads in same CPU.
// As `lock()` is only called when there are no race conditions caused by interrupts.
unsafe impl<T: ?Sized + Sync, R: Deref<Target = SpinLock<T>> + Sync> Sync for SpinLockGuard_<T, R> {}
