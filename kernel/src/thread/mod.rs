// SPDX-License-Identifier: MPL-2.0

//! Posix thread implementation

use core::sync::atomic::Ordering;

use ostd::{
    cpu::{AtomicCpuSet, CpuSet},
    task::Task,
};

use self::status::{AtomicThreadStatus, ThreadStatus};
use crate::{
    prelude::*,
    sched::priority::{AtomicPriority, Priority},
};

pub mod exception;
pub mod kernel_thread;
pub mod oops;
pub mod status;
pub mod task;
pub mod work_queue;

pub type Tid = u32;

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
    status: AtomicThreadStatus,
    /// Thread priority
    priority: AtomicPriority,
    /// Thread CPU affinity
    cpu_affinity: AtomicCpuSet,
}

impl Thread {
    /// Never call these function directly
    pub fn new(
        task: Weak<Task>,
        data: impl Send + Sync + Any,
        status: ThreadStatus,
        priority: Priority,
        cpu_affinity: CpuSet,
    ) -> Self {
        Thread {
            task,
            data: Box::new(data),
            status: AtomicThreadStatus::new(status),
            priority: AtomicPriority::new(priority),
            cpu_affinity: AtomicCpuSet::new(cpu_affinity),
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
    pub fn run(&self) {
        self.status.store(ThreadStatus::Running, Ordering::Release);
        self.task.upgrade().unwrap().run();
    }

    /// Returns whether the thread is exited.
    pub fn is_exited(&self) -> bool {
        self.status.load(Ordering::Acquire).is_exited()
    }

    /// Returns whether the thread is stopped.
    pub fn is_stopped(&self) -> bool {
        self.status.load(Ordering::Acquire).is_stopped()
    }

    /// Stops the thread if it is running.
    ///
    /// If the previous status is not [`ThreadStatus::Running`], this function
    /// returns [`Err`] with the previous state. Otherwise, it sets the status
    /// to [`ThreadStatus::Stopped`] and returns [`Ok`] with the previous state.
    ///
    /// This function only sets the status to [`ThreadStatus::Stopped`],
    /// without initiating a reschedule.
    pub fn stop(&self) -> core::result::Result<ThreadStatus, ThreadStatus> {
        self.status.compare_exchange(
            ThreadStatus::Running,
            ThreadStatus::Stopped,
            Ordering::AcqRel,
            Ordering::Acquire,
        )
    }

    /// Resumes running the thread if it is stopped.
    ///
    /// If the previous status is not [`ThreadStatus::Stopped`], this function
    /// returns [`None`]. Otherwise, it sets the status to
    /// [`ThreadStatus::Running`] and returns [`Some(())`].
    ///
    /// This function only sets the status to [`ThreadStatus::Running`],
    /// without initiating a reschedule.
    pub fn resume(&self) -> Option<()> {
        self.status
            .compare_exchange(
                ThreadStatus::Stopped,
                ThreadStatus::Running,
                Ordering::AcqRel,
                Ordering::Acquire,
            )
            .ok()
            .map(|_| ())
    }

    pub(super) fn exit(&self) {
        self.status.store(ThreadStatus::Exited, Ordering::Release);
    }

    /// Returns the reference to the atomic priority.
    pub fn atomic_priority(&self) -> &AtomicPriority {
        &self.priority
    }

    /// Returns the reference to the atomic CPU affinity.
    pub fn atomic_cpu_affinity(&self) -> &AtomicCpuSet {
        &self.cpu_affinity
    }

    /// Yields the execution to another thread.
    ///
    /// This method will return once the current thread is scheduled again.
    pub fn yield_now() {
        Task::yield_now()
    }

    /// Joins the execution of the thread.
    ///
    /// This method will return after the thread exits.
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
