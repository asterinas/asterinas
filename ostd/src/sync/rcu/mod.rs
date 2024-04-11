// SPDX-License-Identifier: MPL-2.0

//! Read-copy update (RCU).

use core::{
    marker::PhantomData,
    ops::Deref,
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

/// Read-Copy Update Synchronization Mechanism
///
/// # Overview
///
/// RCU avoids the use of lock primitives lock primitives while multiple threads
/// concurrently read and update elements that are linked through pointers and that
/// belong to shared data structures.
///
/// Whenever a thread is inserting or deleting elements of data structures in shared
/// memory, all readers are guaranteed to see and traverse either the older or the
/// new structure, therefore avoiding inconsistencies and allowing readers to not be
/// blocked by writers.
///
/// The type parameter `P` represents the data that this rcu is protecting. The type
/// parameter `P` must implement [`OwnerPtr`].
///
/// # Usage
///
/// It is used when performance of reads is crucial and is an example of space–time
/// tradeoff, enabling fast operations at the cost of more space.
///
/// Use [`Rcu`] in scenarios that require frequent reads and infrequent
/// updates (read-mostly).
///
/// Use [`Rcu`] in scenarios that require high real-time reading.
///
/// Rcu should not to be used in the scenarios that write-mostly and which need
/// consistent data.
///
/// # Examples
///
/// ```
/// use aster_frame::sync::{Rcu, RcuReadGuard, RcuReclaimer};
///
/// let rcu = Rcu::new(Box::new(42));
///
/// // Read the data protected by rcu
/// {
///     let rcu_guard = rcu.get();
///     assert_eq!(*rcu_guard, 42);
/// }
///
/// // Update the data protected by rcu
/// {
///     let reclaimer = rcu.replace(Box::new(43));
///
///     let rcu_guard = rcu.get();
///     assert_eq!(*rcu_guard, 43);
/// }
/// ```
pub struct Rcu<P: OwnerPtr> {
    ptr: AtomicPtr<<P as OwnerPtr>::Target>,
    marker: PhantomData<P::Target>,
}

impl<P: OwnerPtr> Rcu<P> {
    /// Creates a new instance of Rcu with the given pointer.
    pub fn new(ptr: P) -> Self {
        let ptr = AtomicPtr::new(OwnerPtr::into_raw(ptr) as *mut _);
        Self {
            ptr,
            marker: PhantomData,
        }
    }

    /// Retrieves a read guard for the RCU mechanism.
    ///
    /// This method returns a `RcuReadGuard` which allows read-only access to the
    /// underlying data protected by the RCU mechanism.
    pub fn get(&self) -> RcuReadGuard<'_, P> {
        let guard = disable_preempt();
        let obj = unsafe { &*self.ptr.load(Acquire) };
        RcuReadGuard {
            obj,
            _rcu: self,
            _inner_guard: guard,
        }
    }
}

impl<P: OwnerPtr + Send> Rcu<P> {
    /// Replaces the current pointer with a new pointer.
    pub fn replace(&self, new_ptr: P) {
        let new_ptr = <P as OwnerPtr>::into_raw(new_ptr) as *mut _;
        let old_ptr = {
            let old_raw_ptr = self.ptr.swap(new_ptr, AcqRel);
            // SAFETY: It is valid because it was previously returned by `into_raw`.
            unsafe { <P as OwnerPtr>::from_raw(old_raw_ptr) }
        };

        let rcu_monitor = RCU_MONITOR.get().unwrap();
        rcu_monitor.after_grace_period(move || {
            drop(old_ptr);
        });
    }
}

/// A guard that allows read-only access to the data protected by the RCU
/// mechanism.
///
/// Note that the data read can be outdated if the data is updated by another
/// task after acquiring the guard.
pub struct RcuReadGuard<'a, P: OwnerPtr> {
    obj: &'a <P as OwnerPtr>::Target,
    _rcu: &'a Rcu<P>,
    _inner_guard: DisabledPreemptGuard,
}

impl<P: OwnerPtr> Deref for RcuReadGuard<'_, P> {
    type Target = <P as OwnerPtr>::Target;

    fn deref(&self) -> &Self::Target {
        self.obj
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
