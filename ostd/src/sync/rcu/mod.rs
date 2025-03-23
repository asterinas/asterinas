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
use crate::task::{atomic_mode::AsAtomicModeGuard, disable_preempt, DisabledPreemptGuard};

mod monitor;
mod owner_ptr;

pub use owner_ptr::OwnerPtr;

/// A Read-Copy Update (RCU) cell for sharing a pointer between threads.
///
/// The pointer should be a owning pointer with type `P`, which implements
/// [`OwnerPtr`]. For example, `P` can be `Box<T>` or `Arc<T>`.
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
pub struct Rcu<P: OwnerPtr>(RcuInner<P>);

/// A guard that allows access to the pointed data protected by a [`Rcu`].
#[clippy::has_significant_drop]
#[must_use]
pub struct RcuReadGuard<'a, P: OwnerPtr>(RcuReadGuardInner<'a, P>);

/// A Read-Copy Update (RCU) cell for sharing a _nullable_ pointer.  
///
/// This is a variant of [`Rcu`] that allows the contained pointer to be null.
/// So that it can implement `Rcu<Option<P>>` where `P` is not a nullable
/// pointer. It is the same as [`Rcu`] in other aspects.
///
/// # Examples
///
/// ```
/// use ostd::sync::RcuOption;
///
/// static RCU: RcuOption<Box<usize>> = RcuOption::new_none();
///
/// assert!(RCU.read().is_none());
///
/// RCU.update(Box::new(42));
///
/// // Read the data protected by RCU
/// {
///     let rcu_guard = RCU.read().try_get().unwrap();
///     assert_eq!(*rcu_guard, 42);
/// }
///
/// // Update the data protected by RCU
/// {
///     let rcu_guard = RCU.read().try_get().unwrap();
///
///     rcu_guard.compare_exchange(Box::new(43)).unwrap();
///
///     let rcu_guard = RCU.read().try_get().unwrap();
///     assert_eq!(*rcu_guard, 43);
/// }
/// ```
pub struct RcuOption<P: OwnerPtr>(RcuInner<P>);

/// A guard that allows access to the pointed data protected by a [`RcuOption`].
#[clippy::has_significant_drop]
#[must_use]
pub struct RcuOptionReadGuard<'a, P: OwnerPtr>(RcuReadGuardInner<'a, P>);

/// The inner implementation of both [`Rcu`] and [`RcuOption`].
struct RcuInner<P: OwnerPtr> {
    ptr: AtomicPtr<<P as OwnerPtr>::Target>,
    // We want to implement Send and Sync explicitly.
    // Having a pointer field prevents them from being implemented
    // automatically by the compiler.
    _marker: PhantomData<*const P::Target>,
}

// SAFETY: It is apparent that if `P::Target` is `Send`, then `Rcu<P>` is `Send`.
unsafe impl<P: OwnerPtr> Send for RcuInner<P> where <P as OwnerPtr>::Target: Send {}

// SAFETY: To implement `Sync` for `Rcu<P>`, we need to meet two conditions:
//  1. `P::Target` must be `Sync` because `Rcu::get` allows concurrent access.
//  2. `P::Target` must be `Send` because `Rcu::update` may obtain an object
//     of `P` created on another thread.
unsafe impl<P: OwnerPtr> Sync for RcuInner<P>
where
    <P as OwnerPtr>::Target: Send + Sync,
    P: Send,
{
}

impl<P: OwnerPtr> RcuInner<P> {
    const fn new_none() -> Self {
        Self {
            ptr: AtomicPtr::new(core::ptr::null_mut()),
            _marker: PhantomData,
        }
    }

    fn new(pointer: P) -> Self {
        let ptr = <P as OwnerPtr>::into_raw(pointer).as_ptr();
        let ptr = AtomicPtr::new(ptr);
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    fn update(&self, new_ptr: Option<P>) {
        let new_ptr = if let Some(new_ptr) = new_ptr {
            <P as OwnerPtr>::into_raw(new_ptr).as_ptr()
        } else {
            core::ptr::null_mut()
        };

        let old_raw_ptr = self.ptr.swap(new_ptr, AcqRel);

        if let Some(p) = NonNull::new(old_raw_ptr) {
            // SAFETY: It was previously returned by `into_raw`.
            unsafe { delay_drop::<P>(p) };
        }
    }

    fn read(&self) -> RcuReadGuardInner<'_, P> {
        let guard = disable_preempt();
        RcuReadGuardInner {
            obj_ptr: self.ptr.load(Acquire),
            rcu: self,
            _inner_guard: guard,
        }
    }

    fn read_with<'a>(
        &'a self,
        guard: &'a dyn AsAtomicModeGuard,
    ) -> Option<&'a <P as OwnerPtr>::Target> {
        // Ensure that a real atomic-mode guard is obtained.
        let _atomic_mode_guard = guard.as_atomic_mode_guard();

        let obj_ptr = self.ptr.load(Acquire);
        if obj_ptr.is_null() {
            return None;
        }
        // SAFETY:
        // 1. This pointer is not NULL.
        // 2. The `_atomic_mode_guard` guarantees atomic mode for the duration of
        //    lifetime `'a`, the pointer is valid because other writers won't release
        //    the allocation until this task passes the quiescent state.
        Some(unsafe { &*obj_ptr })
    }
}

impl<P: OwnerPtr> Drop for RcuInner<P> {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Acquire);
        if let Some(p) = NonNull::new(ptr) {
            // SAFETY: It was previously returned by `into_raw` when creating
            // the RCU primitive.
            let pointer = unsafe { <P as OwnerPtr>::from_raw(p) };
            // It is OK not to delay the drop because the RCU primitive is
            // owned by nobody else.
            drop(pointer);
        }
    }
}

/// The inner implementation of both [`RcuReadGuard`] and [`RcuOptionReadGuard`].
struct RcuReadGuardInner<'a, P: OwnerPtr> {
    obj_ptr: *mut <P as OwnerPtr>::Target,
    rcu: &'a RcuInner<P>,
    _inner_guard: DisabledPreemptGuard,
}

impl<P: OwnerPtr> RcuReadGuardInner<'_, P> {
    fn compare_exchange(self, new_ptr: Option<P>) -> Result<(), Option<P>> {
        let new_ptr = if let Some(new_ptr) = new_ptr {
            <P as OwnerPtr>::into_raw(new_ptr).as_ptr()
        } else {
            core::ptr::null_mut()
        };

        if self
            .rcu
            .ptr
            .compare_exchange(self.obj_ptr, new_ptr, AcqRel, Acquire)
            .is_err()
        {
            let Some(new_ptr) = NonNull::new(new_ptr) else {
                return Err(None);
            };
            // SAFETY:
            // 1. It was previously returned by `into_raw`.
            // 2. The `compare_exchange` fails so the pointer will not
            //    be used by other threads via reading the RCU primitive.
            return Err(Some(unsafe { <P as OwnerPtr>::from_raw(new_ptr) }));
        }

        if let Some(p) = NonNull::new(self.obj_ptr) {
            // SAFETY: It was previously returned by `into_raw`.
            unsafe { delay_drop::<P>(p) };
        }

        Ok(())
    }
}

impl<P: OwnerPtr> Rcu<P> {
    /// Creates a new RCU primitive with the given pointer.
    pub fn new(pointer: P) -> Self {
        Self(RcuInner::new(pointer))
    }

    /// Replaces the current pointer with a null pointer.
    ///
    /// This function updates the pointer to the new pointer regardless of the
    /// original pointer. The original pointer will be dropped after the grace
    /// period.
    ///
    /// Oftentimes this function is not recommended unless you have serialized
    /// writes with locks. Otherwise, you can use [`Self::read`] and then
    /// [`RcuReadGuard::compare_exchange`] to update the pointer.
    pub fn update(&self, new_ptr: P) {
        self.0.update(Some(new_ptr));
    }

    /// Retrieves a read guard for the RCU primitive.
    ///
    /// The guard allows read access to the data protected by RCU, as well
    /// as the ability to do compare-and-exchange.
    pub fn read(&self) -> RcuReadGuard<'_, P> {
        RcuReadGuard(self.0.read())
    }

    /// Reads the RCU-protected value in an atomic mode.
    ///
    /// The RCU mechanism protects reads ([`Self::read`]) by entering an
    /// atomic mode. If we are already in an atomic mode, this function can
    /// reduce the overhead of disabling preemption again.
    ///
    /// Unlike [`Self::read`], this function does not return a read guard, so
    /// you cannot use [`RcuReadGuard::compare_exchange`] to synchronize the
    /// writers. You may do it via a [`super::SpinLock`].
    pub fn read_with<'a>(
        &'a self,
        guard: &'a dyn AsAtomicModeGuard,
    ) -> &'a <P as OwnerPtr>::Target {
        self.0.read_with(guard).unwrap()
    }
}

impl<P: OwnerPtr> RcuOption<P> {
    /// Creates a new RCU primitive with the given pointer.
    pub fn new(pointer: Option<P>) -> Self {
        if let Some(pointer) = pointer {
            Self(RcuInner::new(pointer))
        } else {
            Self(RcuInner::new_none())
        }
    }

    /// Creates a new RCU primitive that contains nothing.
    ///
    /// This is a constant equivalence to [`RcuOption::new(None)`].
    pub const fn new_none() -> Self {
        Self(RcuInner::new_none())
    }

    /// Replaces the current pointer with a null pointer.
    ///
    /// This function updates the pointer to the new pointer regardless of the
    /// original pointer. If the original pointer is not NULL, it will be
    /// dropped after the grace period.
    ///
    /// Oftentimes this function is not recommended unless you have
    /// synchronized writes with locks. Otherwise, you can use [`Self::read`]
    /// and then [`RcuOptionReadGuard::compare_exchange`] to update the pointer.
    pub fn update(&self, new_ptr: Option<P>) {
        self.0.update(new_ptr);
    }

    /// Retrieves a read guard for the RCU primitive.
    ///
    /// The guard allows read access to the data protected by RCU, as well
    /// as the ability to do compare-and-exchange.
    ///
    /// The contained pointer can be NULL and you can only get a reference
    /// (if checked non-NULL) via [`RcuOptionReadGuard::get`].
    pub fn read(&self) -> RcuOptionReadGuard<'_, P> {
        RcuOptionReadGuard(self.0.read())
    }

    /// Reads the RCU-protected value in an atomic mode.
    ///
    /// The RCU mechanism protects reads ([`Self::read`]) by entering an
    /// atomic mode. If we are already in an atomic mode, this function can
    /// reduce the overhead of disabling preemption again.
    ///
    /// Unlike [`Self::read`], this function does not return a read guard, so
    /// you cannot use [`RcuOptionReadGuard::compare_exchange`] to synchronize the
    /// writers. You may do it via a [`super::SpinLock`].
    pub fn read_with<'a>(
        &'a self,
        guard: &'a dyn AsAtomicModeGuard,
    ) -> Option<&'a <P as OwnerPtr>::Target> {
        self.0.read_with(guard)
    }
}

// RCU guards that have a non-null pointer can be directly dereferenced.
impl<P: OwnerPtr> Deref for RcuReadGuard<'_, P> {
    type Target = <P as OwnerPtr>::Target;

    fn deref(&self) -> &Self::Target {
        // SAFETY:
        // 1. This pointer is not NULL because the type is `RcuReadGuard`.
        // 2. Since the preemption is disabled, the pointer is valid because
        //    other writers won't release the allocation until this task passes
        //    the quiescent state.
        unsafe { &*self.0.obj_ptr }
    }
}

impl<P: OwnerPtr> RcuReadGuard<'_, P> {
    /// Tries to replace the already read pointer with a new pointer.
    ///
    /// If another thread has updated the pointer after the read, this
    /// function will fail, and returns the given pointer back. Otherwise,
    /// it will replace the pointer with the new one and drop the old pointer
    /// after the grace period.
    ///
    /// If spinning on [`Rcu::read`] and this function, it is recommended
    /// to relax the CPU or yield the task on failure. Otherwise contention
    /// will occur.
    ///
    /// This API does not help to avoid
    /// [the ABA problem](https://en.wikipedia.org/wiki/ABA_problem).
    pub fn compare_exchange(self, new_ptr: P) -> Result<(), P> {
        self.0
            .compare_exchange(Some(new_ptr))
            .map_err(|err| err.unwrap())
    }
}

// RCU guards that may have a null pointer can be dereferenced after checking.
impl<P: OwnerPtr> RcuOptionReadGuard<'_, P> {
    /// Gets the reference of the protected data.
    ///
    /// If the RCU primitive protects nothing, this function returns `None`.
    pub fn get(&self) -> Option<&<P as OwnerPtr>::Target> {
        if self.0.obj_ptr.is_null() {
            return None;
        }
        // SAFETY:
        // 1. This pointer is not NULL.
        // 2. Since the preemption is disabled, the pointer is valid because
        //    other writers won't release the allocation until this task passes
        //    the quiescent state.
        Some(unsafe { &*self.0.obj_ptr })
    }

    /// Returns if the RCU primitive protects nothing when [`Rcu::read`] happens.
    pub fn is_none(&self) -> bool {
        self.0.obj_ptr.is_null()
    }

    /// Tries to replace the already read pointer with a new pointer
    /// (or none).
    ///
    /// If another thread has updated the pointer after the read, this
    /// function will fail, and returns the given pointer back. Otherwise,
    /// it will replace the pointer with the new one and drop the old pointer
    /// after the grace period.
    ///
    /// If spinning on [`RcuOption::read`] and this function, it is recommended
    /// to relax the CPU or yield the task on failure. Otherwise contention
    /// will occur.
    ///
    /// This API does not help to avoid
    /// [the ABA problem](https://en.wikipedia.org/wiki/ABA_problem).
    pub fn compare_exchange(self, new_ptr: Option<P>) -> Result<(), Option<P>> {
        self.0.compare_exchange(new_ptr)
    }
}

/// # Safety
///
/// The pointer must be previously returned by `into_raw` and the pointer
/// must be only be dropped once.
unsafe fn delay_drop<P: OwnerPtr>(pointer: NonNull<<P as OwnerPtr>::Target>) {
    struct ForceSend<P: OwnerPtr>(NonNull<<P as OwnerPtr>::Target>);
    // SAFETY: Sending a raw pointer to another task is safe as long as
    // the pointer access in another task is safe (guaranteed by the trait
    // bound `P: Send`).
    unsafe impl<P: OwnerPtr> Send for ForceSend<P> {}

    let pointer: ForceSend<P> = ForceSend(pointer);

    after_grace_period(move || {
        // This is necessary to make the Rust compiler to move the entire
        // `ForceSend` structure into the closure.
        let pointer = pointer;

        // SAFETY:
        // 1. The pointer was previously returned by `into_raw`.
        // 2. The pointer won't be used anymore since the grace period has
        //    finished and this is the only time the pointer gets dropped.
        let p = unsafe { <P as OwnerPtr>::from_raw(pointer.0) };
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
pub(crate) unsafe fn finish_grace_period() {
    let rcu_monitor = RCU_MONITOR.get().unwrap();
    // SAFETY: The caller ensures safety.
    unsafe {
        rcu_monitor.finish_grace_period();
    }
}

/// Registers a callback to be invoked after the current grace period.
pub(crate) fn after_grace_period<F: FnOnce() + Send + 'static>(callback: F) {
    let rcu_monitor = RCU_MONITOR.get().unwrap();
    rcu_monitor.after_grace_period(callback);
}

static RCU_MONITOR: Once<RcuMonitor> = Once::new();

pub fn init() {
    RCU_MONITOR.call_once(RcuMonitor::new);
}
