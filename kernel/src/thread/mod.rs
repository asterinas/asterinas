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
        Task::current()?
            .data()
            .downcast_ref::<Arc<Thread>>()
            .cloned()
    }

    /// Returns the task associated with this thread.
    pub fn task(&self) -> Arc<Task> {
        self.task.upgrade().unwrap()
    }

    /// Gets the Thread from task's data.
    ///
    /// # Panics
    ///
    /// This method panics if the task is not a thread.
    pub fn borrow_from_task(task: &Task) -> &Arc<Self> {
        task.data().downcast_ref::<Arc<Thread>>().unwrap()
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

    pub fn yield_now() {
        Task::yield_now()
    }

    /// Returns the associated data.
    ///
    /// The return type must be borrowed box, otherwise the `downcast_ref` will fail.
    #[allow(clippy::borrowed_box)]
    pub fn data(&self) -> &Box<dyn Send + Sync + Any> {
        &self.data
    }
}
