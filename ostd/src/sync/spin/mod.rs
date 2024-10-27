// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

pub(crate) mod mcs;

use core::{cell::UnsafeCell, fmt, marker::PhantomData};

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
    lock: mcs::LockBody,
    val: UnsafeCell<T>,
}

/// A guardian that denotes the guard behavior for holding the spin lock.
pub trait Guardian {
    /// The guard type.
    type Guard;

    /// Creates a new guard.
    fn guard() -> Self::Guard;
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
            lock: mcs::LockBody::new(),
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
    /// Acquires the spin lock and applies the closure to the inner data.
    ///
    /// If the lock is already held by another thread, the current thread will
    /// spin until the lock is released.
    pub fn lock_with<F, R>(&self, f: F) -> R
    where
        F: FnOnce(&mut T) -> R,
    {
        let _guard = G::guard();

        let mcs_unsafe_node = core::pin::pin!(mcs::UnsafeNode::new());
        let mcs_node = mcs::Node::new(&self.inner.lock, mcs_unsafe_node);

        let mcs_node = mcs_node.lock();

        // SAFETY: The lock is acquired so the critical section is safe.
        let r = f(unsafe { &mut *self.inner.val.get() });

        mcs_node.unlock();

        r
    }

    /// Tries acquiring the spin lock immedidately and applies the closure to
    /// the inner data.
    ///
    /// If the lock is already held by another thread, this method will return
    /// `None`. Otherwise, it will return `Some` with the result of the closure.
    pub fn try_lock_with<F, R>(&self, f: F) -> Option<R>
    where
        F: FnOnce(&mut T) -> R,
    {
        let _guard = G::guard();

        let mcs_unsafe_node = core::pin::pin!(mcs::UnsafeNode::new());
        let mcs_node = mcs::Node::new(&self.inner.lock, mcs_unsafe_node);

        match mcs_node.try_lock() {
            Ok(mcs_node) => {
                // SAFETY: The lock is acquired so the critical section is safe.
                let r = f(unsafe { &mut *self.inner.val.get() });

                mcs_node.unlock();

                Some(r)
            }
            Err(_) => None,
        }
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
