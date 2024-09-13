// SPDX-License-Identifier: MPL-2.0

//! Posix thread implementation

use core::sync::atomic::Ordering;

use ostd::task::Task;

use self::status::{AtomicThreadStatus, ThreadStatus};
use crate::prelude::*;

pub mod exception;
pub mod kernel_thread;
pub mod status;
pub mod task;
pub mod work_queue;

pub type Tid = u32;

/// A thread is a wrapper on top of task.
pub struct Thread {
    // immutable part
    /// Low-level info
    task: Weak<Task>,
    /// Data: Posix thread info/Kernel thread Info
    data: Box<dyn Send + Sync + Any>,

    // mutable part
    status: AtomicThreadStatus,
}

impl Thread {
    /// Never call these function directly
    pub fn new(task: Weak<Task>, data: impl Send + Sync + Any, status: ThreadStatus) -> Self {
        Thread {
            task,
            data: Box::new(data),
            status: AtomicThreadStatus::new(status),
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

    /// Gets the Thread from task's data.
    ///
    /// # Panics
    ///
    /// This method panics if the task is not a thread.
    pub fn borrow_from_task(task: &Arc<Task>) -> &Arc<Self> {
        task.data().downcast_ref::<Arc<Thread>>().unwrap()
    }

    /// Runs this thread at once.
    pub fn run(&self) {
        self.set_status(ThreadStatus::Running);
        self.task.upgrade().unwrap().run();
    }

    pub(super) fn exit(&self) {
        self.set_status(ThreadStatus::Exited);
    }

    /// Returns the reference to the atomic status.
    pub fn atomic_status(&self) -> &AtomicThreadStatus {
        &self.status
    }

    /// Returns the current status.
    pub fn status(&self) -> ThreadStatus {
        self.status.load(Ordering::Acquire)
    }

    /// Updates the status with the `new` value.
    pub fn set_status(&self, new_status: ThreadStatus) {
        self.status.store(new_status, Ordering::Release);
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
