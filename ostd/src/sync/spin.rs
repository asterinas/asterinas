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
    task::{disable_preempt, DisablePreemptGuard},
    trap::{disable_local, DisabledLocalIrqGuard},
};

/// A spin lock.
#[repr(C)]
pub struct SpinLock<T: ?Sized, G = PreemptDisabled> {
    phantom: PhantomData<G>,
    lock: AtomicBool,
    val: UnsafeCell<T>,
}

pub trait Guardian {
    type Guard;

    /// Return a guard object
    fn guard() -> Self::Guard;

    fn inner_guard() -> InnerGuard;
}

/// for lock disabling preempt
pub struct PreemptDisabled;

impl Guardian for PreemptDisabled {
    type Guard = DisablePreemptGuard;

    fn guard() -> Self::Guard {
        disable_preempt()
    }

    fn inner_guard() -> InnerGuard {
        InnerGuard::PreemptGuard(Self::guard())
    }
}

/// for lock disabling Irq
pub struct LocalIrqDisabled;

impl Guardian for LocalIrqDisabled {
    type Guard = DisabledLocalIrqGuard;

    fn guard() -> Self::Guard {
        disable_local()
    }

    fn inner_guard() -> InnerGuard {
        InnerGuard::IrqGuard(Self::guard())
    }
}

impl<T, G: Guardian> SpinLock<T, G> {
    /// Creates a new spin lock.
    pub const fn new(val: T) -> Self {
        Self {
            lock: AtomicBool::new(false),
            val: UnsafeCell::new(val),
            phantom: PhantomData,
        }
    }
}

impl<T: ?Sized> SpinLock<T, PreemptDisabled> {
    /// convert spinlock from PreemptDisabled to LocalIrqDisabled
    pub fn disable_irq(&self) -> &SpinLock<T, LocalIrqDisabled> {
        // SAFETY: The memory layout of the source and target types, which only differ in the `phantom` field, are identical because
        // (1) `PhantomData` is zero-sized, (2) the struct layout follows `repr(C)`.
        unsafe { core::mem::transmute(self) }
    }
}

impl<T: ?Sized, G: Guardian> SpinLock<T, G> {
    /// Acquires the spin lock without disabling local IRQs.
    ///
    /// This method is twice as fast as the [`disable_irq().lock`] method.
    /// So prefer using this method over the [`disable_irq().lock`] method
    /// when IRQ handlers are allowed to get executed while
    /// holding this lock. For example, if a lock is never used
    /// in the interrupt context, then it is ok to use this method
    /// in the process context.
    ///
    /// [`disable_irq().lock`]: Self::disable_irq().lock
    pub fn lock(&self) -> SpinLockGuard<T, G> {
        self.acquire_lock();
        SpinLockGuard_ {
            lock: self,
            inner_guard: G::inner_guard(),
        }
    }

    /// Acquires the spin lock through an [`Arc`].
    ///
    /// The method is similar to [`lock`], but it doesn't have the requirement
    /// for compile-time checked lifetimes of the lock guard.
    ///
    /// [`lock`]: Self::lock
    pub fn lock_arc(self: &Arc<Self>) -> ArcSpinLockGuard<T, G> {
        self.acquire_lock();
        SpinLockGuard_ {
            lock: self.clone(),
            inner_guard: G::inner_guard(),
        }
    }

    /// Tries acquiring the spin lock immedidately without disabling the local IRQs.
    pub fn try_lock(&self) -> Option<SpinLockGuard<T, G>> {
        if self.try_acquire_lock() {
            let lock_guard = SpinLockGuard_ {
                lock: self,
                inner_guard: G::inner_guard(),
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

impl<T: ?Sized + fmt::Debug, G> fmt::Debug for SpinLock<T, G> {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        fmt::Debug::fmt(&self.val, f)
    }
}

// SAFETY: Only a single lock holder is permitted to access the inner data of Spinlock.
unsafe impl<T: ?Sized + Send, G> Send for SpinLock<T, G> {}
unsafe impl<T: ?Sized + Send, G> Sync for SpinLock<T, G> {}

pub enum InnerGuard {
    IrqGuard(DisabledLocalIrqGuard),
    PreemptGuard(DisablePreemptGuard),
}

/// A guard that provides exclusive access to the data protected by a [`SpinLock`].
pub type SpinLockGuard<'a, T, G> = SpinLockGuard_<T, &'a SpinLock<T, G>, G>;
/// A guard that provides exclusive access to the data protected by a `Arc<SpinLock>`.
pub type ArcSpinLockGuard<T, G> = SpinLockGuard_<T, Arc<SpinLock<T, G>>, G>;

/// The guard of a spin lock that disables the local IRQs.
#[clippy::has_significant_drop]
#[must_use]
pub struct SpinLockGuard_<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> {
    inner_guard: InnerGuard,
    lock: R,
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> Deref for SpinLockGuard_<T, R, G> {
    type Target = T;

    fn deref(&self) -> &T {
        unsafe { &*self.lock.val.get() }
    }
}

impl<T: ?Sized, R: Deref<Target = SpinLock<T, G>>, G: Guardian> DerefMut
    for SpinLockGuard_<T, R, G>
{
    fn deref_mut(&mut self) -> &mut Self::Target {
        unsafe { &mut *self.lock.val.get() }
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
