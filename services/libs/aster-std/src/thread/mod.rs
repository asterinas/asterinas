//! Posix thread implementation

use core::{
    any::Any,
    sync::atomic::{AtomicU32, Ordering},
};

use aster_frame::task::Task;

use crate::prelude::*;

use self::status::ThreadStatus;

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
    status: Mutex<ThreadStatus>,
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
            status: Mutex::new(status),
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

    /// Run this thread at once.
    pub fn run(&self) {
        self.status.lock().set_running();
        self.task.run();
    }

    pub fn exit(&self) {
        let mut status = self.status.lock();
        if !status.is_exited() {
            status.set_exited();
        }
    }

    pub fn is_exited(&self) -> bool {
        self.status.lock().is_exited()
    }

    pub fn status(&self) -> &Mutex<ThreadStatus> {
        &self.status
    }

    pub fn yield_now() {
        Task::yield_now()
    }

    pub fn tid(&self) -> Tid {
        self.tid
    }

    // The return type must be borrowed box, otherwise the downcast_ref will fail
    #[allow(clippy::borrowed_box)]
    pub fn data(&self) -> &Box<dyn Send + Sync + Any> {
        &self.data
    }
}

/// allocate a new pid for new process
pub fn allocate_tid() -> Tid {
    TID_ALLOCATOR.fetch_add(1, Ordering::SeqCst)
}
