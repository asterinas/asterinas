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
};

mod monitor;
mod owner_ptr;

pub use owner_ptr::OwnerPtr;

pub struct Rcu<P: OwnerPtr> {
    ptr: AtomicPtr<<P as OwnerPtr>::Target>,
    marker: PhantomData<P::Target>,
}

impl<P: OwnerPtr> Rcu<P> {
    pub fn new(ptr: P) -> Self {
        let ptr = AtomicPtr::new(OwnerPtr::into_raw(ptr) as *mut _);
        Self {
            ptr,
            marker: PhantomData,
        }
    }

    pub fn get(&self) -> RcuReadGuard<'_, P> {
        let obj = unsafe { &*self.ptr.load(Acquire) };
        RcuReadGuard { obj, rcu: self }
    }
}

impl<P: OwnerPtr + Send> Rcu<P> {
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
