// SPDX-License-Identifier: MPL-2.0

//! Posix thread implementation

use core::sync::atomic::{AtomicBool, Ordering};

use ostd::{
    cpu::{AtomicCpuSet, CpuSet},
    task::Task,
};

use crate::{
    prelude::*,
    sched::{SchedAttr, SchedPolicy},
};

pub mod exception;
pub mod kernel_thread;
pub mod oops;
pub mod task;
pub mod work_queue;

pub type Tid = u32;

fn pre_schedule_handler() {
    let Some(task) = Task::current() else {
        return;
    };
    let Some(thread_local) = task.as_thread_local() else {
        return;
    };

    thread_local.fpu().before_schedule();
}

fn post_schedule_handler() -> bool {
    let Some(task) = Task::current() else {
        return true;
    };
    let Some(thread_local) = task.as_thread_local() else {
        return true;
    };

    let root_vmar = thread_local.root_vmar().borrow();
    let res = if let Some(vmar) = root_vmar.as_ref() {
        vmar.vm_space().activate()
    } else {
        true
    };

    thread_local.fpu().after_schedule();

    #[cfg(feature = "breakdown_counters")]
    crate::fs::procfs::breakdown_counters::sched_end();

    res
}

pub(super) fn init() {
    ostd::task::inject_pre_schedule_handler(pre_schedule_handler);
    ostd::task::inject_post_schedule_handler(post_schedule_handler);
    #[cfg(feature = "breakdown_counters")]
    ostd::task::scheduler::inject_pre_schedule_handler(
        crate::fs::procfs::breakdown_counters::sched_start,
    );
    #[cfg(feature = "breakdown_counters")]
    ostd::task::scheduler::inject_schedule_do_nothing_handler(
        crate::fs::procfs::breakdown_counters::sched_end,
    );
    ostd::arch::trap::inject_user_page_fault_handler(exception::page_fault_handler);
}

/// A thread is a wrapper on top of task.
#[derive(Debug)]
pub struct Thread {
    // immutable part
    /// Low-level info
    task: Weak<Task>,
    /// Data: Posix thread info/Kernel thread Info
    data: Box<dyn Send + Sync + Any>,

    // mutable part
    /// Thread status
    is_exited: AtomicBool,
    /// Thread CPU affinity
    cpu_affinity: AtomicCpuSet,
    sched_attr: SchedAttr,
}

impl Thread {
    /// Never call these function directly
    pub fn new(
        task: Weak<Task>,
        data: impl Send + Sync + Any,
        cpu_affinity: CpuSet,
        sched_policy: SchedPolicy,
    ) -> Self {
        Thread {
            task,
            data: Box::new(data),
            is_exited: AtomicBool::new(false),
            cpu_affinity: AtomicCpuSet::new(cpu_affinity),
            sched_attr: SchedAttr::new(sched_policy),
        }
    }

    /// Returns the current thread.
    ///
    /// This function returns `None` if the current task is not associated with
    /// a thread, or if called within the bootstrap context.
    pub fn current() -> Option<Arc<Self>> {
        Task::current()?.as_thread().cloned()
    }

    /// Returns the task associated with this thread.
    pub fn task(&self) -> Arc<Task> {
        self.task.upgrade().unwrap()
    }

    /// Runs this thread at once.
    #[track_caller]
    pub fn run(&self) {
        self.task.upgrade().unwrap().run();
    }

    /// Returns whether the thread is exited.
    pub fn is_exited(&self) -> bool {
        self.is_exited.load(Ordering::Acquire)
    }

    pub(super) fn exit(&self) {
        self.is_exited.store(true, Ordering::Release);
    }

    /// Returns the reference to the atomic CPU affinity.
    pub fn atomic_cpu_affinity(&self) -> &AtomicCpuSet {
        &self.cpu_affinity
    }

    pub fn sched_attr(&self) -> &SchedAttr {
        &self.sched_attr
    }

    /// Yields the execution to another thread.
    ///
    /// This method will return once the current thread is scheduled again.
    #[track_caller]
    pub fn yield_now() {
        #[cfg(feature = "breakdown_counters")]
        crate::fs::procfs::breakdown_counters::sched_start();
        Task::yield_now()
    }

    /// Joins the execution of the thread.
    ///
    /// This method will return after the thread exits.
    #[track_caller]
    pub fn join(&self) {
        while !self.is_exited() {
            Self::yield_now();
        }
    }

    /// Returns the associated data.
    pub fn data(&self) -> &(dyn Send + Sync + Any) {
        &*self.data
    }
}

/// A trait to provide the `as_thread` method for tasks.
pub trait AsThread {
    /// Returns the associated [`Thread`].
    fn as_thread(&self) -> Option<&Arc<Thread>>;
}

impl AsThread for Task {
    fn as_thread(&self) -> Option<&Arc<Thread>> {
        self.data().downcast_ref::<Arc<Thread>>()
    }
}
