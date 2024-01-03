// SPDX-License-Identifier: MPL-2.0

use crate::prelude::*;
use crate::sync::SpinLock;
use crate::task::Task;

use lazy_static::lazy_static;

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
