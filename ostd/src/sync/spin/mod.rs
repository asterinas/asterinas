// SPDX-License-Identifier: MPL-2.0

pub(crate) mod queued;

use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use super::{guard::SpinGuardian, LocalIrqDisabled, PreemptDisabled};
use crate::task::atomic_mode::AsAtomicModeGuard;

/// A spin lock.
///
/// # Guard behavior
///
/// The type `G' specifies the guard behavior of the spin lock. While holding the lock,
/// - if `G` is [`PreemptDisabled`], preemption is disabled;
/// - if `G` is [`LocalIrqDisabled`], local IRQs are disabled.
///
/// The `G` can also be provided by other crates other than ostd,
/// if it behaves similar like [`PreemptDisabled`] or [`LocalIrqDisabled`].
///
/// The guard behavior can be temporarily upgraded from [`PreemptDisabled`] to
/// [`LocalIrqDisabled`] using the [`disable_irq`] method.
///
/// [`disable_irq`]: Self::disable_irq
#[repr(transparent)]
pub struct SpinLock<T: ?Sized, G = PreemptDisabled> {
    phantom: PhantomData<G>,
    /// Only the last field of a struct may have a dynamically sized type.
    /// That's why SpinLockInner is put in the last field.
    inner: SpinLockInner<T>,
}

struct SpinLockInner<T: ?Sized> {
    lock: queued::LockBody,
    val: UnsafeCell<T>,
}

impl<T, G> SpinLock<T, G> {
    /// Creates a new spin lock.
    pub const fn new(val: T) -> Self {
        let lock_inner = SpinLockInner {
            lock: queued::LockBody::new(),
            val: UnsafeCell::new(val),
        };
        Self {
            phantom: PhantomData,
            inner: lock_inner,
        }
    }
}

impl<T: ?Sized> SpinLock<T, PreemptDisabled> {
    /// Converts the guard behavior from disabling preemption to disabling IRQs.
    pub fn disable_irq(&self) -> &SpinLock<T, LocalIrqDisabled> {
        let ptr = self as *const SpinLock<T, PreemptDisabled>;
        let ptr = ptr as *const SpinLock<T, LocalIrqDisabled>;
        // SAFETY:
        // 1. The types `SpinLock<T, PreemptDisabled>`, `SpinLockInner<T>` and `SpinLock<T,
        //    IrqDisabled>` have the same memory layout guaranteed by `#[repr(transparent)]`.
        // 2. The specified memory location can be borrowed as an immutable reference for the
        //    specified lifetime.
        unsafe { &*ptr }
    }
}

impl<T: ?Sized, G: SpinGuardian> SpinLock<T, G> {
    /// Acquires the spin lock.
    pub fn lock(&self) -> SpinLockGuard<T, G> {
        let guard = G::guard();

        self.inner.lock.lock(guard.as_atomic_mode_guard());

        SpinLockGuard { lock: self, guard }
    }

    /// Tries acquiring the spin lock immedidately.
    pub fn try_lock(&self) -> Option<SpinLockGuard<T, G>> {
        let guard = G::guard();
        if self.inner.lock.try_lock(guard.as_atomic_mode_guard()) {
            Some(SpinLockGuard { lock: self, guard })
        } else {
            None
        }
    }

    /// Returns a mutable reference to the underlying data.
    ///
    /// This method is zero-cost: By holding a mutable reference to the lock, the compiler has
    /// already statically guaranteed that access to the data is exclusive.
    pub fn get_mut(&mut self) -> &mut T {
        self.inner.val.get_mut()
    }
}

impl<T: ?Sized + fmt::Debug, G> fmt::Debug for SpinLock<T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.inner.val, f)
    }
}

// SAFETY: Only a single lock holder is permitted to access the inner data of Spinlock.
unsafe impl<T: ?Sized + Send, G> Send for SpinLock<T, G> {}
unsafe impl<T: ?Sized + Send, G> Sync for SpinLock<T, G> {}

/// The guard of a spin lock.
#[clippy::has_significant_drop]
#[must_use]
pub struct SpinLockGuard<'a, T: ?Sized, G: SpinGuardian> {
    guard: G::Guard,
    lock: &'a SpinLock<T, G>,
}

impl<T: ?Sized, G: SpinGuardian> AsAtomicModeGuard for SpinLockGuard<'_, T, G> {
    fn as_atomic_mode_guard(&self) -> &dyn crate::task::atomic_mode::InAtomicMode {
        self.guard.as_atomic_mode_guard()
    }
}

impl<T: ?Sized, G: SpinGuardian> Deref for SpinLockGuard<'_, T, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> DerefMut for SpinLockGuard<'_, T, G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, G: SpinGuardian> Drop for SpinLockGuard<'_, T, G> {
    fn drop(&mut self) {
        // SAFETY:
        //  - We do not move the lock since the guard takes a reference to the
        //    lock.
        //  - Preemption is disabled while holding the lock.
        //  - The lock is locked and not unlocked before calling this function.
        unsafe {
            self.lock.inner.lock.unlock();
        }
    }
}

impl<T: ?Sized + fmt::Debug, G: SpinGuardian> fmt::Debug for SpinLockGuard<'_, T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized, G: SpinGuardian> !Send for SpinLockGuard<'_, T, G> {}

// SAFETY: `SpinLockGuard` can be shared between tasks/threads in same CPU.
// As `lock()` is only called when there are no race conditions caused by interrupts.
unsafe impl<T: ?Sized + Sync, G: SpinGuardian> Sync for SpinLockGuard<'_, T, G> {}
