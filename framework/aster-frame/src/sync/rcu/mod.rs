// SPDX-License-Identifier: MPL-2.0

//! Read-copy update (RCU).

use alloc::sync::Arc;
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
/// Read-Copy Update (RCU) avoids the use of lock primitives lock primitives while
/// multiple threads concurrently read and update elements that are linked through
/// pointers and that belong to shared data structures.
///
/// Whenever a thread is inserting or deleting elements of data structures in shared
/// memory, all readers are guaranteed to see and traverse either the older or the
/// new structure, therefore avoiding inconsistencies and allowing readers to not be
/// blocked by writers.
///
/// The type parameter `P` represents the data that this `Rcu` is protecting. The type
/// parameter `P` must implement `OwnerPtr`.
///
/// # Usage
///
/// It is used when performance of reads is crucial and is an example of space–time
/// tradeoff, enabling fast operations at the cost of more space.
///
/// Use `Rcu` in scenarios that require frequent reads and infrequent updates (read-mostly).
/// Use `Rcu` in scenarios that require high real-time reading.
///
/// RCU should not to be used in the scenarios that write-mostly and which need
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
    /// Creates a new instance of `Rcu` with the given pointer.
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
    /// This function has _subscription semantics_.
    ///
    /// # Safety
    ///
    /// The pointer protected by the `Rcu` must be valid and point to a valid object.
    pub fn get(&self) -> RcuReadGuard<'_, P> {
        let preempt_guard = disable_preempt();
        let obj = unsafe { &*self.ptr.load(Acquire) };
        RcuReadGuard {
            obj,
            rcu: self,
            preempt_guard,
        }
    }
}

impl<P: OwnerPtr> Rcu<P>
where
    P::Target: Clone,
{
    pub fn copy(&self) -> P::Target {
        unsafe { (*self.ptr.load(Acquire)).clone() }
    }
}

impl<P: OwnerPtr + Send> Rcu<P> {
    /// Replaces the current pointer with a new pointer and returns a `RcuReclaimer` that
    /// can be used to reclaim the old pointer.
    ///
    /// This function has _publication semantics_.
    pub fn replace(&self, new_ptr: P) -> RcuReclaimer<P> {
        let new_ptr = <P as OwnerPtr>::into_raw(new_ptr) as *mut _;
        let old_ptr = {
            let old_raw_ptr = self.ptr.swap(new_ptr, AcqRel);
            unsafe { <P as OwnerPtr>::from_raw(old_raw_ptr) }
        };
        RcuReclaimer { ptr: old_ptr }
    }
}

pub struct RcuReadGuard<'a, P: OwnerPtr> {
    obj: &'a <P as OwnerPtr>::Target,
    rcu: &'a Rcu<P>,
    preempt_guard: DisablePreemptGuard,
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
    pub fn delay(self) {
        // SAFETY: The `read` behavior is not undefined because
        // 1) The pointer is valid for reads;
        // 2) The pointer points to a properly initialized value;
        // 3) The pointer will be forgotten rather than being dropped so the value will
        //    be only dropped once by `drop(ptr)`;
        // 4) Before `self` gets forgtten, the code won't return prematurely or panic.
        let ptr = unsafe { core::ptr::read(&self.ptr) };
        core::mem::forget(self);

        let rcu_monitor = RCU_MONITOR.get().unwrap().lock();
        rcu_monitor.after_grace_period(move || {
            drop(ptr);
        });
    }
}

impl<P> Drop for RcuReclaimer<P> {
    fn drop(&mut self) {
        let wait_queue = Arc::new(WaitQueue::new());
        let rcu_monitor = RCU_MONITOR.get().unwrap().lock();
        rcu_monitor.after_grace_period({
            let wait_queue = Arc::clone(&wait_queue);
            move || {
                wait_queue.wake_one();
            }
        });
        // Release the lock held by `rcu_monitor` before calling `wait()` to avoid deadlock.
        drop(rcu_monitor);
        wait_queue.wait();
    }
}

/// Inform the RCU mechanism that the current task has reached a quiescent state.
/// A quiescent state is a point in the execution of a task at which
/// it is guaranteed not to be holding any references to RCU-protected data.
///
/// # Safety
///
/// Determining whether the current task has reached a quiescent state
/// is fundamental for the RCU mechanism to work properly.
/// This responsibility falls on the shoulder of the caller of this function.
/// Failing to do so leads to undefined behaviors.
pub(crate) unsafe fn pass_quiescent_state() {
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
