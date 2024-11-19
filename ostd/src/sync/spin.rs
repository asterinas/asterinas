// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use alloc::sync::Arc;
use core::{
    cell::UnsafeCell,
    fmt,
    marker::PhantomData,
    ops::{Deref, DerefMut},
    sync::atomic::{AtomicBool, Ordering},
};

use crate::{
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
    lock: AtomicBool,
    val: UnsafeCell<T>,
}

/// A guardian that denotes the guard behavior for holding the spin lock.
pub trait Guardian {
    /// The guard type.
    type Guard: GuardTransfer;

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
            lock: AtomicBool::new(false),
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
        // Notice the guard must be created before acquiring the lock.
        let inner_guard = G::guard();
        self.acquire_lock();
        SpinLockGuard_ {
            lock: self,
            guard: inner_guard,
        }
    }

    /// Acquires the spin lock through an [`Arc`].
    ///
    /// The method is similar to [`lock`], but it doesn't have the requirement
    /// for compile-time checked lifetimes of the lock guard.
    ///
    /// [`lock`]: Self::lock
    pub fn lock_arc(self: &Arc<Self>) -> ArcSpinLockGuard<T, G> {
        let inner_guard = G::guard();
        self.acquire_lock();
        SpinLockGuard_ {
            lock: self.clone(),
            guard: inner_guard,
        }
    }

    /// Tries acquiring the spin lock immedidately.
    pub fn try_lock(&self) -> Option<SpinLockGuard<T, G>> {
        let inner_guard = G::guard();
        if self.try_acquire_lock() {
            let lock_guard = SpinLockGuard_ {
                lock: self,
                guard: inner_guard,
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
        self.inner
            .lock
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .is_ok()
    }

    fn release_lock(&self) {
        self.inner.lock.store(false, Ordering::Release);
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

/// A guard that provides exclusive access to the data protected by a [`SpinLock`].
pub type SpinLockGuard<'a, T, G> = SpinLockGuard_<T, &'a SpinLock<T, G>, G>;
/// A guard that provides exclusive access to the data protected by a `Arc<SpinLock>`.
pub type ArcSpinLockGuard<T, G> = SpinLockGuard_<T, Arc<SpinLock<T, G>>, G>;

/// The guard of a spin lock.
#[clippy::has_significant_drop]
#[must_use]
pub struct SpinLockGuard_<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> {
    guard: G::Guard,
    lock: R,
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> Deref for SpinLockGuard_<T, R, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> DerefMut
    for SpinLockGuard_<T, R, G>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.inner.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> Drop for SpinLockGuard_<T, R, G> {
    fn drop(&mut self) {
        self.lock.release_lock();
    }
}

impl<T: ?Sized + fmt::Debug, R: Deref<Target = SpinLock<T, G>>, G: Guardian> fmt::Debug
    for SpinLockGuard_<T, R, G>
{
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&**self, f)
    }
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> !Send for SpinLockGuard_<T, R, G> {}

// SAFETY: `SpinLockGuard_` can be shared between tasks/threads in same CPU.
// As `lock()` is only called when there are no race conditions caused by interrupts.
unsafe impl<T: ?Sized + Sync, R: Deref<Target = SpinLock<T, G>> + Sync, G: Guardian> Sync
    for SpinLockGuard_<T, R, G>
{
}
