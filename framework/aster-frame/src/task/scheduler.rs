// SPDX-License-Identifier: MPL-2.0

use alloc::collections::VecDeque;

use lazy_static::lazy_static;

use crate::{prelude::*, sync::SpinLock, task::Task};

lazy_static! {
    pub(crate) static ref GLOBAL_SCHEDULER: SpinLock<GlobalScheduler> =
        SpinLock::new(GlobalScheduler { scheduler: None });
}

/// A scheduler for tasks.
///
/// An implementation of scheduler can attach scheduler-related information
/// with the `TypeMap` returned from `task.data()`.
pub trait Scheduler: Sync + Send {
    fn enqueue(&self, task: Arc<Task>);

    fn dequeue(&self) -> Option<Arc<Task>>;

    /// Tells whether the given task should be preempted by other tasks in the queue.
    fn should_preempt(&self, task: &Arc<Task>) -> bool;
}

pub struct GlobalScheduler {
    scheduler: Option<&'static dyn Scheduler>,
}

impl GlobalScheduler {
    pub fn new() -> Self {
        Self { scheduler: None }
    }

    /// dequeue a task using scheduler
    /// require the scheduler is not none
    pub fn dequeue(&mut self) -> Option<Arc<Task>> {
        self.scheduler.unwrap().dequeue()
    }
    /// enqueue a task using scheduler
    /// require the scheduler is not none
    pub fn enqueue(&mut self, task: Arc<Task>) {
        self.scheduler.unwrap().enqueue(task)
    }

    pub fn should_preempt(&self, task: &Arc<Task>) -> bool {
        self.scheduler.unwrap().should_preempt(task)
    }
}
/// Set the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: &'static dyn Scheduler) {
    GLOBAL_SCHEDULER.lock_irq_disabled().scheduler = Some(scheduler);
}

pub fn fetch_task() -> Option<Arc<Task>> {
    GLOBAL_SCHEDULER.lock_irq_disabled().dequeue()
}

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
