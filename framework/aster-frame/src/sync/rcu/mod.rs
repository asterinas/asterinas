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
#[cfg(target_arch = "x86_64")]
use crate::arch::x86::cpu;
use crate::{
    prelude::*,
    sync::{SpinLock, WaitQueue},
    task::{disable_preempt, DisablePreemptGuard},
};

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
/// parameter `P` must implement `OwnerPtr`.
///
/// # Usage
/// It is used when performance of reads is crucial and is an example of space–time
/// tradeoff, enabling fast operations at the cost of more space.
///
/// Use `Rcu` in scenarios that require frequent reads and infrequent updates(read-mostly).
/// Use `Rcu` in scenarios that require high real-time reading.
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
///     // Delay the reclamation of the old data
///     reclaimer.delay();
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
    ///
    /// This function has the semantics of _subscribe_ in RCU mechanism.
    ///
    /// # Safety
    ///
    /// The pointer protected by the Rcu must be valid and point to a valid object.
    ///
    /// # Non-Preemptible RCU
    ///
    /// In non-preemptible RCU, the read-side critical section is delimited using
    /// a `PreemptGuard`.
    // TODO: Distinguish different type RCU
    pub fn get(&self) -> RcuReadGuard<'_, P> {
        let guard = disable_preempt();
        let obj = unsafe { &*self.ptr.load(Acquire) };
        RcuReadGuard {
            obj,
            rcu: self,
            inner_guard: InnerGuard::Preempt(guard),
        }
    }
}

impl<P: OwnerPtr + Send> Rcu<P> {
    /// Replaces the current pointer with a new pointer and returns a `RcuReclaimer` that
    /// can be used to reclaim the old pointer.
    ///
    /// This function has the semantics of _publish_ in RCU mechanism.
    pub fn replace(&self, new_ptr: P) -> RcuReclaimer<P> {
        let new_ptr = <P as OwnerPtr>::into_raw(new_ptr) as *mut _;
        let old_ptr = {
            let old_raw_ptr = self.ptr.swap(new_ptr, AcqRel);
            unsafe { <P as OwnerPtr>::from_raw(old_raw_ptr) }
        };
        RcuReclaimer { ptr: old_ptr }
    }
}

enum InnerGuard {
    Preempt(DisablePreemptGuard),
    Empty,
}

pub struct RcuReadGuard<'a, P: OwnerPtr> {
    obj: &'a <P as OwnerPtr>::Target,
    rcu: &'a Rcu<P>,
    inner_guard: InnerGuard,
}

impl<'a, P: OwnerPtr> Deref for RcuReadGuard<'a, P> {
    type Target = <P as OwnerPtr>::Target;

    fn deref(&self) -> &Self::Target {
        self.obj
    }
}

#[repr(transparent)]
pub struct RcuReclaimer<P> {
    ptr: P,
}

impl<P: Send + 'static> RcuReclaimer<P> {
    pub fn delay(mut self) {
        let ptr: P = unsafe {
            let ptr = core::mem::replace(
                &mut self.ptr,
                core::mem::MaybeUninit::uninit().assume_init(),
            );

            core::mem::forget(self);

            ptr
        };

        let rcu_monitor = RCU_MONITOR.get().unwrap().lock();
        rcu_monitor.after_grace_period(move || {
            drop(ptr);
        });
    }
}

impl<P> Drop for RcuReclaimer<P> {
    fn drop(&mut self) {
        let wq = Arc::new(WaitQueue::new());
        let rcu_monitor = RCU_MONITOR.get().unwrap().lock();
        rcu_monitor.after_grace_period({
            let wq = wq.clone();
            move || {
                wq.wake_one();
            }
        });
        wq.wait_until(|| Some(0u8));
    }
}

/// Passes the quiescent state to the singleton RcuMonitor and take the callbacks if
/// the current GP is complete.
///
/// # Non-Preemptible RCU
///
/// This function is commonly used when the thread is in the user state or idle loop,
/// or when calling `schedule()` to indicate that the cpu is entering the quiescent state.
pub unsafe fn pass_quiescent_state() {
    let rcu_monitor = RCU_MONITOR.get().unwrap().lock();
    rcu_monitor.pass_quiescent_state()
}

static RCU_MONITOR: Once<SpinLock<RcuMonitor>> = Once::new();

pub fn init() {
    RCU_MONITOR.call_once(|| {
        let num_cpus = cpu::num_cpus() as usize;
        SpinLock::new(RcuMonitor::new(num_cpus))
    });
}
