// SPDX-License-Identifier: MPL-2.0

//! Posix thread implementation

use core::sync::atomic::{AtomicU32, Ordering};

use ostd::task::{yield_now, Task, YieldFlags};

use self::status::{AtomicThreadStatus, ThreadStatus};
use crate::{
    cpu::CpuSet,
    prelude::*,
    sched::{AtomicPriority, Priority},
};

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
    /// Thread status
    status: AtomicThreadStatus,
    /// Thread priority
    priority: AtomicPriority,
    /// Cpu affinity
    cpu_affinity: SpinLock<CpuSet>,
}

impl Thread {
    /// Never call these function directly
    pub fn new(
        tid: Tid,
        task: Arc<Task>,
        data: impl Send + Sync + Any,
        status: ThreadStatus,
        priority: Priority,
        cpu_affinity: CpuSet,
    ) -> Self {
        Thread {
            tid,
            task,
            data: Box::new(data),
            status: AtomicThreadStatus::new(status),
            priority: AtomicPriority::new(priority),
            cpu_affinity: SpinLock::new(cpu_affinity),
        }
    }

    /// Returns the current thread, or `None` if the current task is not associated with a thread.
    ///
    /// Except for unit tests, all tasks should be associated with threads. This method is useful
    /// when writing code that can be called directly by unit tests. If this isn't the case,
    /// consider using [`current_thread!`] instead.
    pub fn current() -> Option<Arc<Self>> {
        Task::current()
            .data()
            .downcast_ref::<Weak<Thread>>()?
            .upgrade()
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

    /// Returns the reference to the atomic priority.
    pub fn atomic_priority(&self) -> &AtomicPriority {
        &self.priority
    }

    /// Returns the current priority.
    pub fn priority(&self) -> Priority {
        self.priority.load(Ordering::Acquire)
    }

    /// Updates the priority with the `new` value.
    pub fn set_priority(&self, new_priority: Priority) {
        self.priority.store(new_priority, Ordering::Release);
    }

    /// Returns the reference to the atomic cpu affinty.
    pub fn atomic_cpu_affinity(&self) -> &SpinLock<CpuSet> {
        &self.cpu_affinity
    }

    /// Returns the current cpu affinity.
    pub fn cpu_affinity(&self) -> CpuSet {
        self.cpu_affinity.lock_irq_disabled().clone()
    }

    /// Updates the cpu affinity with the `new` value.
    pub fn set_cpu_affinity(&self, new_cpu_affinity: CpuSet) {
        *self.cpu_affinity.lock_irq_disabled() = new_cpu_affinity;
    }

    pub fn yield_now() {
        yield_now(YieldFlags::Yield)
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
