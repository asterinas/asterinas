// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

use alloc::collections::VecDeque;

use crate::{prelude::*, sync::SpinLock, task::Task};

static DEFAULT_SCHEDULER: FifoScheduler = FifoScheduler::new();
pub(crate) static GLOBAL_SCHEDULER: SpinLock<GlobalScheduler> = SpinLock::new(GlobalScheduler {
    scheduler: &DEFAULT_SCHEDULER,
});

/// A scheduler for tasks.
///
/// An implementation of scheduler can attach scheduler-related information
/// with the `TypeMap` returned from `task.data()`.
pub trait Scheduler: Sync + Send {
    /// Enqueues a task to the scheduler.
    fn enqueue(&self, task: Arc<Task>);

    /// Dequeues a task from the scheduler.
    fn dequeue(&self) -> Option<Arc<Task>>;

    /// Tells whether the given task should be preempted by other tasks in the queue.
    fn should_preempt(&self, task: &Arc<Task>) -> bool;
}

pub struct GlobalScheduler {
    scheduler: &'static dyn Scheduler,
}

impl GlobalScheduler {
    pub const fn new(scheduler: &'static dyn Scheduler) -> Self {
        Self { scheduler }
    }

    /// dequeue a task using scheduler
    /// require the scheduler is not none
    pub fn dequeue(&mut self) -> Option<Arc<Task>> {
        self.scheduler.dequeue()
    }
    /// enqueue a task using scheduler
    /// require the scheduler is not none
    pub fn enqueue(&mut self, task: Arc<Task>) {
        self.scheduler.enqueue(task)
    }

    pub fn should_preempt(&self, task: &Arc<Task>) -> bool {
        self.scheduler.should_preempt(task)
    }
}
/// Sets the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: &'static dyn Scheduler) {
    let mut global_scheduler = GLOBAL_SCHEDULER.lock_irq_disabled();
    // When setting a new scheduler, the old scheduler should be empty
    assert!(global_scheduler.dequeue().is_none());
    global_scheduler.scheduler = scheduler;
}

pub fn fetch_task() -> Option<Arc<Task>> {
    GLOBAL_SCHEDULER.lock_irq_disabled().dequeue()
}

/// Adds a task to the global scheduler.
pub fn add_task(task: Arc<Task>) {
    GLOBAL_SCHEDULER.lock_irq_disabled().enqueue(task);
}

/// A simple FIFO (First-In-First-Out) task scheduler.
pub struct FifoScheduler {
    /// A thread-safe queue to hold tasks waiting to be executed.
    task_queue: SpinLock<VecDeque<Arc<Task>>>,
}

impl FifoScheduler {
    /// Creates a new instance of `FifoScheduler`.
    pub const fn new() -> Self {
        FifoScheduler {
            task_queue: SpinLock::new(VecDeque::new()),
        }
    }
}

impl Default for FifoScheduler {
    fn default() -> Self {
        Self::new()
    }
}

impl Scheduler for FifoScheduler {
    /// Enqueues a task to the end of the queue.
    fn enqueue(&self, task: Arc<Task>) {
        self.task_queue.lock_irq_disabled().push_back(task);
    }
    /// Dequeues a task from the front of the queue, if any.
    fn dequeue(&self) -> Option<Arc<Task>> {
        self.task_queue.lock_irq_disabled().pop_front()
    }
    /// In this simple implementation, task preemption is not supported.
    /// Once a task starts running, it will continue to run until completion.
    fn should_preempt(&self, _task: &Arc<Task>) -> bool {
        false
    }
}
