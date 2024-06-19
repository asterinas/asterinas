// SPDX-License-Identifier: MPL-2.0

//! Posix thread implementation

use core::sync::atomic::{AtomicU32, Ordering};

use ostd::task::Task;

use self::status::{AtomicThreadStatus, ThreadStatus};
use crate::prelude::*;

pub mod exception;
pub mod kernel_thread;
pub mod status;
pub mod task;
pub mod thread_table;
pub mod work_queue;

pub type Tid = u32;

static TID_ALLOCATOR: AtomicU32 = AtomicU32::new(0);

/// A thread is a wrapper on top of task.
pub struct Thread {
    // immutable part
    /// Thread id
    tid: Tid,
    /// Low-level info
    task: Arc<Task>,
    /// Data: Posix thread info/Kernel thread Info
    data: Box<dyn Send + Sync + Any>,

    // mutable part
    status: AtomicThreadStatus,
}

impl Thread {
    /// Never call these function directly
    pub fn new(
        tid: Tid,
        task: Arc<Task>,
        data: impl Send + Sync + Any,
        status: ThreadStatus,
    ) -> Self {
        Thread {
            tid,
            task,
            data: Box::new(data),
            status: AtomicThreadStatus::new(status),
        }
    }

    pub fn current() -> Arc<Self> {
        let task = Task::current();
        let thread = task
            .data()
            .downcast_ref::<Weak<Thread>>()
            .expect("[Internal Error] task data should points to weak<thread>");
        thread
            .upgrade()
            .expect("[Internal Error] current thread cannot be None")
    }

    pub(in crate::thread) fn task(&self) -> &Arc<Task> {
        &self.task
    }

    /// Runs this thread at once.
    pub fn run(&self) {
        self.set_status(ThreadStatus::Running);
        self.task.run();
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

    pub fn tid(&self) -> Tid {
        self.tid
    }

    /// Returns the associated data.
    ///
    /// The return type must be borrowed box, otherwise the `downcast_ref` will fail.
    #[allow(clippy::borrowed_box)]
    pub fn data(&self) -> &Box<dyn Send + Sync + Any> {
        &self.data
    }
}

/// Allocates a new tid for the new thread
pub fn allocate_tid() -> Tid {
    TID_ALLOCATOR.fetch_add(1, Ordering::SeqCst)
}
