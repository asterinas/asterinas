// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

pub(crate) mod queued;

use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
};

use crate::{
    cpu::PinCurrentCpu,
    task::{disable_preempt, DisabledPreemptGuard},
    trap::{disable_local, DisabledLocalIrqGuard},
};

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
pub struct SpinLock<T: ?Sized, G: Guardian = PreemptDisabled> {
    phantom: PhantomData<G>,
    /// Only the last field of a struct may have a dynamically sized type.
    /// That's why SpinLockInner is put in the last field.
    inner: SpinLockInner<T>,
}

struct SpinLockInner<T: ?Sized> {
    lock: queued::LockBody,
    val: UnsafeCell<T>,
}

/// A guardian that denotes the guard behavior for holding the spin lock.
pub trait Guardian {
    /// The guard type.
    type Guard: GuardTransfer + PinCurrentCpu;

    /// Creates a new guard.
    fn guard() -> Self::Guard;
}

/// The Guard can be transferred atomically.
pub trait GuardTransfer {
    /// Atomically transfers the current guard to a new instance.
    ///
    /// This function ensures that there are no 'gaps' between the destruction of the old guard and
    /// the creation of the new guard, thereby maintaining the atomicity of guard transitions.
    ///
    /// The original guard must be dropped immediately after calling this method.
    fn transfer_to(&mut self) -> Self;
}

/// A guardian that disables preemption while holding the spin lock.
pub struct PreemptDisabled;

impl Guardian for PreemptDisabled {
    type Guard = DisabledPreemptGuard;

    fn guard() -> Self::Guard {
        disable_preempt()
    }
}

/// A guardian that disables IRQs while holding the spin lock.
///
/// This guardian would incur a certain time overhead over
/// [`PreemptDisabled']. So prefer avoiding using this guardian when
/// IRQ handlers are allowed to get executed while holding the
/// lock. For example, if a lock is never used in the interrupt
/// context, then it is ok not to use this guardian in the process context.
pub struct LocalIrqDisabled;

impl Guardian for LocalIrqDisabled {
    type Guard = DisabledLocalIrqGuard;

    fn guard() -> Self::Guard {
        disable_local()
    }
}

impl<T, G: Guardian> SpinLock<T, G> {
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

impl<T: ?Sized, G: Guardian> SpinLock<T, G> {
    /// Acquires the spin lock.
    pub fn lock(&self) -> SpinLockGuard<T, G> {
        let guard = G::guard();

        // SAFETY: `G::guard()` ensures that the current task is pinned to the
        // current CPU.
        unsafe {
            self.inner.lock.lock();
        }

        SpinLockGuard { lock: self, guard }
    }

    /// Tries acquiring the spin lock immedidately.
    pub fn try_lock(&self) -> Option<SpinLockGuard<T, G>> {
        let guard = G::guard();
        if self.inner.lock.try_lock() {
            Some(SpinLockGuard { lock: self, guard })
        } else {
            None
        }
    }
}

impl<T: ?Sized + fmt::Debug, G: Guardian> fmt::Debug for SpinLock<T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.inner.val, f)
    }
}

// SAFETY: Only a single lock holder is permitted to access the inner data of Spinlock.
unsafe impl<T: ?Sized + Send, G: Guardian> Send for SpinLock<T, G> {}
unsafe impl<T: ?Sized + Send, G: Guardian> Sync for SpinLock<T, G> {}

/// The guard of a spin lock.
#[clippy::has_significant_drop]
#[must_use]
pub struct SpinLockGuard<'a, T: ?Sized, G: Guardian> {
    guard: G::Guard,
    lock: &'a SpinLock<T, G>,
}

impl<T: ?Sized, G: Guardian> Deref for SpinLockGuard<'_, T, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, G: Guardian> DerefMut for SpinLockGuard<'_, T, G> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, G: Guardian> Drop for SpinLockGuard<'_, T, G> {
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

impl<T: ?Sized + fmt::Debug, G: Guardian> fmt::Debug for SpinLockGuard<'_, T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized, G: Guardian> !Send for SpinLockGuard<'_, T, G> {}

// SAFETY: `SpinLockGuard` can be shared between tasks/threads in same CPU.
// As `lock()` is only called when there are no race conditions caused by interrupts.
unsafe impl<T: ?Sized + Sync, G: Guardian> Sync for SpinLockGuard<'_, T, G> {}
