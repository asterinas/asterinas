// SPDX-License-Identifier: MPL-2.0

//! Read-copy update (RCU).

use core::{
    marker::PhantomData,
    mem::ManuallyDrop,
    ops::Deref,
    ptr::NonNull,
    sync::atomic::{
        AtomicPtr,
        Ordering::{AcqRel, Acquire},
    },
};

use non_null::NonNullPtr;
use spin::once::Once;

use self::monitor::RcuMonitor;
use crate::task::{
    atomic_mode::{AsAtomicModeGuard, InAtomicMode},
    disable_preempt, DisabledPreemptGuard,
};

mod monitor;
pub mod non_null;

/// A Read-Copy Update (RCU) cell for sharing a pointer between threads.
///
/// The pointer should be a non-null pointer with type `P`, which implements
/// [`NonNullPtr`]. For example, `P` can be `Box<T>` or `Arc<T>`.
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
pub struct Rcu<P: NonNullPtr>(RcuInner<P>);

/// A guard that allows access to the pointed data protected by a [`Rcu`].
#[clippy::has_significant_drop]
#[must_use]
pub struct RcuReadGuard<'a, P: NonNullPtr>(RcuReadGuardInner<'a, P>);

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
pub struct RcuOption<P: NonNullPtr>(RcuInner<P>);

/// A guard that allows access to the pointed data protected by a [`RcuOption`].
#[clippy::has_significant_drop]
#[must_use]
pub struct RcuOptionReadGuard<'a, P: NonNullPtr>(RcuReadGuardInner<'a, P>);

/// The inner implementation of both [`Rcu`] and [`RcuOption`].
struct RcuInner<P: NonNullPtr> {
    ptr: AtomicPtr<<P as NonNullPtr>::Target>,
    // We want to implement Send and Sync explicitly.
    // Having a pointer field prevents them from being implemented
    // automatically by the compiler.
    _marker: PhantomData<*const P::Target>,
}

// SAFETY: It is apparent that if `P` is `Send`, then `Rcu<P>` is `Send`.
unsafe impl<P: NonNullPtr> Send for RcuInner<P> where P: Send {}

// SAFETY: To implement `Sync` for `Rcu<P>`, we need to meet two conditions:
//  1. `P` must be `Sync` because `Rcu::get` allows concurrent access.
//  2. `P` must be `Send` because `Rcu::update` may obtain an object
//     of `P` created on another thread.
unsafe impl<P: NonNullPtr> Sync for RcuInner<P> where P: Send + Sync {}

impl<P: NonNullPtr + Send> RcuInner<P> {
    const fn new_none() -> Self {
        Self {
            ptr: AtomicPtr::new(core::ptr::null_mut()),
            _marker: PhantomData,
        }
    }

    fn new(pointer: P) -> Self {
        let ptr = <P as NonNullPtr>::into_raw(pointer).as_ptr();
        let ptr = AtomicPtr::new(ptr);
        Self {
            ptr,
            _marker: PhantomData,
        }
    }

    fn update(&self, new_ptr: Option<P>) {
        let new_ptr = if let Some(new_ptr) = new_ptr {
            <P as NonNullPtr>::into_raw(new_ptr).as_ptr()
        } else {
            core::ptr::null_mut()
        };

        let old_raw_ptr = self.ptr.swap(new_ptr, AcqRel);

        if let Some(p) = NonNull::new(old_raw_ptr) {
            // SAFETY:
            // 1. The pointer was previously returned by `into_raw`.
            // 2. The pointer is removed from the RCU slot so that no one will
            //    use it after the end of the current grace period. The removal
            //    is done atomically, so it will only be dropped once.
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

    fn read_with<'a>(&'a self, _guard: &'a dyn InAtomicMode) -> Option<P::Ref<'a>> {
        let obj_ptr = self.ptr.load(Acquire);
        if obj_ptr.is_null() {
            return None;
        }
        // SAFETY:
        // 1. This pointer is not NULL.
        // 2. The `_guard` guarantees atomic mode for the duration of lifetime
        //    `'a`, the pointer is valid because other writers won't release the
        //    allocation until this task passes the quiescent state.
        NonNull::new(obj_ptr).map(|ptr| unsafe { P::raw_as_ref(ptr) })
    }
}

impl<P: NonNullPtr> Drop for RcuInner<P> {
    fn drop(&mut self) {
        let ptr = self.ptr.load(Acquire);
        if let Some(p) = NonNull::new(ptr) {
            // SAFETY: It was previously returned by `into_raw` when creating
            // the RCU primitive.
            let pointer = unsafe { <P as NonNullPtr>::from_raw(p) };
            // It is OK not to delay the drop because the RCU primitive is
            // owned by nobody else.
            drop(pointer);
        }
    }
}

/// The inner implementation of both [`RcuReadGuard`] and [`RcuOptionReadGuard`].
struct RcuReadGuardInner<'a, P: NonNullPtr> {
    obj_ptr: *mut <P as NonNullPtr>::Target,
    rcu: &'a RcuInner<P>,
    _inner_guard: DisabledPreemptGuard,
}

impl<P: NonNullPtr + Send> RcuReadGuardInner<'_, P> {
    fn get(&self) -> Option<P::Ref<'_>> {
        // SAFETY: The guard ensures that `P` will not be dropped. Thus, `P`
        // outlives the lifetime of `&self`. Additionally, during this period,
        // it is impossible to create a mutable reference to `P`.
        NonNull::new(self.obj_ptr).map(|ptr| unsafe { P::raw_as_ref(ptr) })
    }

    fn compare_exchange(self, new_ptr: Option<P>) -> Result<(), Option<P>> {
        let new_ptr = if let Some(new_ptr) = new_ptr {
            <P as NonNullPtr>::into_raw(new_ptr).as_ptr()
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
            return Err(Some(unsafe { <P as NonNullPtr>::from_raw(new_ptr) }));
        }

        if let Some(p) = NonNull::new(self.obj_ptr) {
            // SAFETY:
            // 1. The pointer was previously returned by `into_raw`.
            // 2. The pointer is removed from the RCU slot so that no one will
            //    use it after the end of the current grace period. The removal
            //    is done atomically, so it will only be dropped once.
            unsafe { delay_drop::<P>(p) };
        }

        Ok(())
    }
}

impl<P: NonNullPtr + Send> Rcu<P> {
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
    pub fn read_with<'a, G: AsAtomicModeGuard + ?Sized>(&'a self, guard: &'a G) -> P::Ref<'a> {
        self.0.read_with(guard.as_atomic_mode_guard()).unwrap()
    }
}

impl<P: NonNullPtr + Send> RcuOption<P> {
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
    pub fn read_with<'a, G: AsAtomicModeGuard + ?Sized>(
        &'a self,
        guard: &'a G,
    ) -> Option<P::Ref<'a>> {
        self.0.read_with(guard.as_atomic_mode_guard())
    }
}

impl<P: NonNullPtr + Send> RcuReadGuard<'_, P> {
    /// Gets the reference of the protected data.
    pub fn get(&self) -> P::Ref<'_> {
        self.0.get().unwrap()
    }

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

impl<P: NonNullPtr + Send> RcuOptionReadGuard<'_, P> {
    /// Gets the reference of the protected data.
    ///
    /// If the RCU primitive protects nothing, this function returns `None`.
    pub fn get(&self) -> Option<P::Ref<'_>> {
        self.0.get()
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

/// Delays the dropping of a [`NonNullPtr`] after the RCU grace period.
///
/// This is internally needed for implementing [`Rcu`] and [`RcuOption`]
/// because we cannot alias a [`Box`]. Restoring `P` and use [`RcuDrop`] for it
/// can lead to multiple [`Box`]es simultaneously pointing to the same
/// content.
///
/// # Safety
///
/// The pointer must be previously returned by `into_raw`, will not be used
/// after the end of the current grace period, and will only be dropped once.
///
/// [`Box`]: alloc::boxed::Box
unsafe fn delay_drop<P: NonNullPtr + Send>(pointer: NonNull<<P as NonNullPtr>::Target>) {
    struct ForceSend<P: NonNullPtr + Send>(NonNull<<P as NonNullPtr>::Target>);
    // SAFETY: Sending a raw pointer to another task is safe as long as
    // the pointer access in another task is safe (guaranteed by the trait
    // bound `P: Send`).
    unsafe impl<P: NonNullPtr + Send> Send for ForceSend<P> {}

    let pointer: ForceSend<P> = ForceSend(pointer);

    let rcu_monitor = RCU_MONITOR.get().unwrap();
    rcu_monitor.after_grace_period(move || {
        // This is necessary to make the Rust compiler to move the entire
        // `ForceSend` structure into the closure.
        let pointer = pointer;

        // SAFETY:
        // 1. The pointer was previously returned by `into_raw`.
        // 2. The pointer won't be used anymore since the grace period has
        //    finished and this is the only time the pointer gets dropped.
        let p = unsafe { <P as NonNullPtr>::from_raw(pointer.0) };
        drop(p);
    });
}

/// A wrapper to delay calling destructor of `T` after the RCU grace period.
///
/// Upon dropping this structure, a callback will be registered to the global
/// RCU monitor and the destructor of `T` will be delayed until the callback.
///
/// [`RcuDrop<T>`] is guaranteed to have the same layout as `T`. You can also
/// access the inner value safely via [`RcuDrop<T>`].
#[derive(Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord, Hash)]
#[repr(transparent)]
pub struct RcuDrop<T: Send + 'static> {
    value: ManuallyDrop<T>,
}

impl<T: Send + 'static> RcuDrop<T> {
    /// Creates a new [`RcuDrop`] instance that delays the dropping of `value`.
    pub fn new(value: T) -> Self {
        Self {
            value: ManuallyDrop::new(value),
        }
    }
}

impl<T: Send + 'static> Deref for RcuDrop<T> {
    type Target = T;

    fn deref(&self) -> &Self::Target {
        &self.value
    }
}

impl<T: Send + 'static> Drop for RcuDrop<T> {
    fn drop(&mut self) {
        // SAFETY: The `ManuallyDrop` will not be used after this point.
        let taken = unsafe { ManuallyDrop::take(&mut self.value) };
        let rcu_monitor = RCU_MONITOR.get().unwrap();
        rcu_monitor.after_grace_period(|| {
            drop(taken);
        });
    }
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
    let rcu_monitor = RCU_MONITOR.get().unwrap();
    // SAFETY: The caller ensures safety.
    unsafe {
        rcu_monitor.finish_grace_period();
    }
}

static RCU_MONITOR: Once<RcuMonitor> = Once::new();

pub fn init() {
    RCU_MONITOR.call_once(RcuMonitor::new);
}
