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
use crate::task::{disable_preempt, DisabledPreemptGuard};

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
/// The constant parameter `NULLABLE` specifies whether the RCU cell can
/// contain a `None` variant. This is by default `false`. If it is `true`,
/// please refer to [`RcuOption`] (a type alias) for more information.
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
#[repr(transparent)]
pub struct Rcu<P: OwnerPtr>(RcuInner<P>);

/// A guard that allows access to the pointed data protected a [`Rcu`].
#[repr(transparent)]
pub struct RcuReadGuard<'a, P: OwnerPtr>(RcuReadGuardInner<'a, P>);

/// Nullable Read-Copy Update Synchronization Cell.
///
/// This is a variant of [`Rcu`] that allows the contained object to be none.
/// It is the same as [`Rcu`] in other aspects.
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

/// A guard that allows access to the pointed data protected a [`RcuOption`].
#[repr(transparent)]
pub struct RcuOptionReadGuard<'a, P: OwnerPtr>(RcuReadGuardInner<'a, P>);

/// A inner implementation of both [`Rcu`] and [`RcuOption`].
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

impl<P: OwnerPtr + Send> RcuInner<P> {
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
}

/// A inner implementation of both [`RcuReadGuard`] and [`RcuOptionReadGuard`].
struct RcuReadGuardInner<'a, P: OwnerPtr> {
    obj_ptr: *mut <P as OwnerPtr>::Target,
    rcu: &'a RcuInner<P>,
    _inner_guard: DisabledPreemptGuard,
}

impl<P: OwnerPtr + Send> RcuReadGuardInner<'_, P> {
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
            //    be used by other threads via reading the RCU cell.
            return Err(Some(unsafe { <P as OwnerPtr>::from_raw(new_ptr) }));
        }

        if let Some(p) = NonNull::new(self.obj_ptr) {
            // SAFETY: It was previously returned by `into_raw`.
            unsafe { delay_drop::<P>(p) };
        }

        Ok(())
    }
}

impl<P: OwnerPtr + Send> Rcu<P> {
    /// Creates a new RCU cell with the given pointer.
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
    pub fn update(&self, new_ptr: Option<P>) {
        self.0.update(new_ptr);
    }

    /// Retrieves a read guard for the RCU cell.
    ///
    /// The guard allows read access to the data protected by RCU, as well
    /// as the ability to do compare-and-exchange.
    pub fn read(&self) -> RcuReadGuard<'_, P> {
        RcuReadGuard(self.0.read())
    }
}

impl<P: OwnerPtr + Send> RcuOption<P> {
    /// Creates a new RCU cell with the given pointer.
    pub fn new(pointer: Option<P>) -> Self {
        if let Some(pointer) = pointer {
            Self(RcuInner::new(pointer))
        } else {
            Self(RcuInner::new_none())
        }
    }

    /// Creates a new RCU cell that contains nothing.
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

    /// Retrieves a read guard for the RCU cell.
    ///
    /// The guard allows read access to the data protected by RCU, as well
    /// as the ability to do compare-and-exchange.
    ///
    /// The contained pointer nullable and you can only dereference it after
    /// checking with [`RcuOptionReadGuard::try_get`].
    pub fn read(&self) -> RcuOptionReadGuard<'_, P> {
        RcuOptionReadGuard(self.0.read())
    }
}

// RCU guards that have a non-null pointer can be directly dereferenced.
impl<P: OwnerPtr + Send> Deref for RcuReadGuard<'_, P> {
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

// RCU guards that may have a null pointer can be dereferenced after checking.
impl<'a, P: OwnerPtr + Send> RcuOptionReadGuard<'a, P> {
    /// Tries to get the non-null read guard.
    ///
    /// If the RCU cell contains none, this function will return [`Err`] with
    /// the guard itself unchanged. Otherwise a dereferenceable read guard will
    /// be returned.
    pub fn try_get(self) -> Result<RcuReadGuard<'a, P>, Self> {
        if self.0.obj_ptr.is_null() {
            return Err(self);
        }
        // SAFETY: The layout of `RcuReadGuard` and `RcuOptionReadGuard` are
        // the same. We have checked that the pointer is not NULL, so the
        // pointer satisfies the requirements of `RcuReadGuard`.
        Ok(unsafe { core::mem::transmute::<Self, RcuReadGuard<'a, P>>(self) })
    }

    /// Returns if the RCU cell contains none when doing [`Rcu::read`].
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

impl<P: OwnerPtr + Send> RcuReadGuard<'_, P> {
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

/// # Safety
///
/// The pointer must be previously returned by `into_raw` and the pointer
/// must be only be dropped once.
unsafe fn delay_drop<P: OwnerPtr + Send>(pointer: NonNull<<P as OwnerPtr>::Target>) {
    struct ForceSend<P: OwnerPtr>(NonNull<<P as OwnerPtr>::Target>);
    // SAFETY: Sending a raw pointer to another task is safe as long as
    // the pointer access in another task is safe (guaranteed by the trait
    // bound `P: Send`).
    unsafe impl<P: OwnerPtr + Send> Send for ForceSend<P> {}

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
