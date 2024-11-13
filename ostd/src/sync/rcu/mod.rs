// SPDX-License-Identifier: MPL-2.0

//! Read-copy update (RCU).

use core::{
    marker::PhantomData,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{
        AtomicPtr,
        Ordering::{AcqRel, Acquire},
    },
};

use spin::once::Once;

use self::monitor::RcuMonitor;
use crate::{
    sync::SpinLock,
    task::{disable_preempt, DisabledPreemptGuard},
};

mod monitor;
mod owner_ptr;

pub use owner_ptr::OwnerPtr;

/// Read-Copy Update Synchronization Cell.
///
/// # Overview
///
/// Read-Copy-Update (RCU) is a synchronization mechanism designed for high-
/// performance, low-latency read operations in concurrent systems. It allows
/// multiple readers to access shared data simultaneously without contention,
/// while writers can update the data safely in a way that does not disrupt
/// ongoing reads. RCU is particularly suited for situations where reads are
/// far more frequent than writes.  
///  
/// The original design and implementation of RCU is described in paper _The
/// Read-Copy-Update Mechanism for Supporting Real-Time Applications on Shared-
/// Memory Multiprocessor Systems with Linux_ published on IBM Systems Journal
/// 47.2 (2008).
///
/// The type parameter `P` represents the data that this RCU is protecting. The
/// type parameter `P` must implement [`OwnerPtr`].
///
/// # Examples
///
/// ```
/// use ostd::sync::Rcu;
///
/// let rcu = Rcu::new(Box::new(42));
///
/// let rcu_guard = rcu.read();
///
/// assert_eq!(*rcu_guard, Some(&42));
///
/// rcu_guard.compare_exchange(Box::new(43)).unwrap();
///
/// let rcu_guard = rcu.read();
///
/// assert_eq!(*rcu_guard, Some(&43));
/// ```
pub type Rcu<P> = Rcu_<P, false>;

/// Lazily Initialized Read-Copy Update Synchronization Cell.
///
/// This is a variant of [`Rcu`] that allows lazy initialization. It is the
/// same as [`Rcu`] in other aspects.
///
/// # Examples
///
/// ```
/// use ostd::sync::LazyRcu;
///
/// // Also allows lazy initialization.
/// static RCU: LazyRcu<Box<usize>> = LazyRcu::new_uninit();
///
/// // Not initialized yet.
/// {
///     assert!(RCU.read().try_get().is_none());
/// }
///
/// // Initialize the data protected by RCU.
/// RCU.update(Box::new(42));
///
/// // Read the data protected by RCU
/// {
///     let rcu_guard = RCU.read_maybe_uninit().try_get().unwrap();
///     assert_eq!(*rcu_guard, 42);
/// }
///
/// // Update the data protected by RCU
/// {
///     let rcu_guard = RCU.read_maybe_uninit().try_get().unwrap();
///
///     rcu_guard.compare_exchange(Box::new(43)).unwrap();
///
///     let rcu_guard = RCU.read_maybe_uninit().try_get().unwrap();
///     assert_eq!(*rcu_guard, 43);
/// }
/// ```
pub type LazyRcu<P> = Rcu_<P, true>;

/// Maybe Uninitialized Read-Copy Update Synchronization Cell.
///
/// This type implements both initialized and lazy RCU objects. Oftentimes this
/// is not preferred. See [`Rcu`] and [`LazyRcu`].
///
// The representation must be transparent to allow us to assume a maybe
// uninitialized RCU object initialized.
#[repr(transparent)]
pub struct Rcu_<P: OwnerPtr, const MAYBE_UNINIT: bool> {
    ptr: AtomicPtr<<P as OwnerPtr>::Target>,
    // We want to implement Send and Sync explicitly.
    // Having a pointer field prevents them from being implemented
    // automatically by the compiler.
    _marker: PhantomData<*const P::Target>,
}

// SAFETY. It is apparent that if `P::Target` is `Send`, then `Rcu<P>` is `Send`.
unsafe impl<P: OwnerPtr, const MAYBE_UNINIT: bool> Send for Rcu_<P, MAYBE_UNINIT> where
    <P as OwnerPtr>::Target: Send
{
}

// SAFETY. To implement `Sync` for `Rcu<P>`, we need to meet two conditions:
//  1. `P::Target` must be `Sync` because `Rcu::get` allows concurrent access.
//  2. `P::Target` must be `Send` because `Rcu::replace` may obtain an object
//     of `P` created on another thread.
unsafe impl<P: OwnerPtr, const MAYBE_UNINIT: bool> Sync for Rcu_<P, MAYBE_UNINIT> where
    <P as OwnerPtr>::Target: Send + Sync
{
}

// Initialized RCU object.
impl<P: OwnerPtr> Rcu_<P, false> {
    /// Creates a new RCU object with the given pointer.
    pub fn new(pointer: P) -> Self {
        let ptr = <P as OwnerPtr>::into_raw(pointer).cast_mut();
        let ptr = AtomicPtr::new(ptr);
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Retrieves a read guard for the RCU object.
    ///
    /// The guard allows read-only access to the data protected by RCU.
    pub fn read(&self) -> RcuReadGuard<'_, P, false> {
        let guard = disable_preempt();
        RcuReadGuard {
            obj_ptr: self.ptr.load(Acquire),
            rcu: self,
            _inner_guard: guard,
        }
    }
}

// Maybe uninitialized RCU object.
impl<P: OwnerPtr> Rcu_<P, true> {
    /// Creates a new uninitialized RCU object.
    ///
    /// Initialization can be done by calling
    /// [`RcuReadGuard::compare_exchange`] after getting a read
    /// guard using [`Rcu_::read`]. Then only the first initialization will be
    /// successful. If initialization can be done multiple times, using
    /// [`Rcu_::update`] is fine.
    pub const fn new_uninit() -> Self {
        let ptr = AtomicPtr::new(core::ptr::null_mut());
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    /// Retrieves a read guard for the RCU object.
    ///
    /// The guard allows read-only access to the data protected by RCU. If the
    /// data is not initialized, the guard will behave as `None` before getting
    /// dereferenced.
    pub fn read_maybe_uninit(&self) -> RcuReadGuard<'_, P, true> {
        let guard = disable_preempt();
        RcuReadGuard {
            obj_ptr: self.ptr.load(Acquire),
            rcu: self,
            _inner_guard: guard,
        }
    }
}

impl<P: OwnerPtr + Send, const MAYBE_UNINIT: bool> Rcu_<P, MAYBE_UNINIT> {
    /// Replaces the current pointer with a new pointer.
    ///
    /// This function updates the pointer to the new pointer regardless of the
    /// original pointer. If the original pointer is not NULL, it will be
    /// dropped after the grace period.
    pub fn update(&self, new_ptr: P) {
        let new_ptr = <P as OwnerPtr>::into_raw(new_ptr).cast_mut();
        let old_raw_ptr = self.ptr.swap(new_ptr, AcqRel);

        if let Some(p) = NonNull::new(old_raw_ptr) {
            // SAFETY: It was previously returned by `into_raw`.
            unsafe { delay_drop::<P>(p) };
        }
    }
}

/// A guard that allows read-only access to the initialized data protected
/// by the RCU mechanism.
pub struct RcuReadGuard<'a, P: OwnerPtr, const MAYBE_UNINIT: bool> {
    /// If maybe uninitialized, the pointer can be NULL.
    obj_ptr: *mut <P as OwnerPtr>::Target,
    rcu: &'a Rcu_<P, MAYBE_UNINIT>,
    _inner_guard: DisabledPreemptGuard,
}

// Initialized RCU guard can be dereferenced.
impl<P: OwnerPtr> Deref for RcuReadGuard<'_, P, false> {
    type Target = <P as OwnerPtr>::Target;

    fn deref(&self) -> &Self::Target {
        // SAFETY: Since the preemption is disabled, the pointer is valid
        // because other writers won't update the pointer until this task
        // passes the quiescent state.
        // And this pointer is not NULL.
        unsafe { &*self.obj_ptr }
    }
}

// Maybe uninitialized RCU guard can be dereferenced after checking.
impl<'a, P: OwnerPtr> RcuReadGuard<'a, P, true> {
    /// Tries to get the initialized read guard.
    ///
    /// If the RCU object is not initialized, this function will return
    /// [`Err`] with the guard itself unchanged. Otherwise a dereferenceable
    /// read guard will be returned.
    pub fn try_get(self) -> Result<RcuReadGuard<'a, P, false>, Self> {
        if self.obj_ptr.is_null() {
            return Err(self);
        }
        Ok(RcuReadGuard {
            obj_ptr: self.obj_ptr,
            // SAFETY: It is initialized. The layout is the same.
            rcu: unsafe { core::mem::transmute::<&Rcu_<P, true>, &Rcu_<P, false>>(self.rcu) },
            _inner_guard: self._inner_guard,
        })
    }
}

impl<P: OwnerPtr + Send, const MAYBE_UNINIT: bool> RcuReadGuard<'_, P, MAYBE_UNINIT> {
    /// Tries to replace the already read pointer with a new pointer.
    ///
    /// If another thread has updated the pointer after the read, this
    /// function will fail and return the new pointer. Otherwise, it will
    /// replace the pointer with the new one and drop the old pointer after
    /// the grace period.
    ///
    /// If spinning on this function, it is recommended to relax the CPU
    /// or yield the task on failure. Otherwise contention will occur.
    ///
    /// This API does not help to avoid
    /// [the ABA problem](https://en.wikipedia.org/wiki/ABA_problem).
    pub fn compare_exchange(self, new_ptr: P) -> Result<(), P> {
        let new_ptr = <P as OwnerPtr>::into_raw(new_ptr).cast_mut();

        if self
            .rcu
            .ptr
            .compare_exchange(self.obj_ptr, new_ptr, AcqRel, Acquire)
            .is_err()
        {
            // SAFETY: It was previously returned by `into_raw`.
            return Err(unsafe { <P as OwnerPtr>::from_raw(new_ptr) });
        }

        if let Some(p) = NonNull::new(self.obj_ptr) {
            // SAFETY: It was previously returned by `into_raw`.
            unsafe { delay_drop::<P>(p) };
        }

        Ok(())
    }
}

/// # Safety
///
/// The pointer must be previously returned by `into_raw` and the pointer
/// must be only be dropped once.
unsafe fn delay_drop<P: OwnerPtr + Send>(pointer: NonNull<<P as OwnerPtr>::Target>) {
    // SAFETY: The pointer is not NULL.
    let p = unsafe { <P as OwnerPtr>::from_raw(pointer.as_ptr().cast_const()) };
    let rcu_monitor = RCU_MONITOR.get().unwrap().lock();
    rcu_monitor.after_grace_period(move || {
        drop(p);
    });
}

/// Finishes the current grace period.
///
/// This function is called when the current grace period on current CPU is
/// finished. If this CPU is the last CPU to finish the current grace period,
/// it takes all the current callbacks and invokes them.
///
/// # Safety
///
/// The caller must ensure that this CPU is not executing in a RCU read-side
/// critical section.
pub unsafe fn finish_grace_period() {
    let rcu_monitor = RCU_MONITOR.get().unwrap().lock();
    // SAFETY: The caller ensures safety.
    unsafe {
        rcu_monitor.finish_grace_period();
    }
}

static RCU_MONITOR: Once<SpinLock<RcuMonitor>> = Once::new();

pub fn init() {
    RCU_MONITOR.call_once(|| SpinLock::new(RcuMonitor::new()));
}
