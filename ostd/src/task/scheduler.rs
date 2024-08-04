// SPDX-License-Identifier: MPL-2.0

use crate::{prelude::*, sync::SpinLock, task::Task};

pub(crate) static GLOBAL_SCHEDULER: SpinLock<Option<Box<dyn Scheduler>>> = SpinLock::new(None);

/// A scheduler for tasks.
///
/// An implementation of scheduler can attach scheduler-related information
/// with the `TypeMap` returned from `task.data()`.
pub trait Scheduler: Sync + Send {
    /// Enqueues a task to the scheduler.
    fn enqueue(&mut self, task: Arc<Task>);

    /// Dequeues a task from the scheduler.
    fn dequeue(&mut self) -> Option<Arc<Task>>;

    /// Tells whether the given task should be preempted by other tasks in the queue.
    fn should_preempt(&mut self, task: &Arc<Task>) -> bool;
}

/// Sets the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: Box<dyn Scheduler>) {
    let mut global_scheduler = GLOBAL_SCHEDULER.lock_irq_disabled();
    // When setting a new scheduler, the old scheduler should be empty
    assert!(global_scheduler.is_none() || global_scheduler.as_mut().unwrap().dequeue().is_none());
    *global_scheduler = Some(scheduler);
}

pub fn fetch_task() -> Option<Arc<Task>> {
    GLOBAL_SCHEDULER
        .lock_irq_disabled()
        .as_mut()
        .unwrap()
        .dequeue()
}

/// Adds a task to the global scheduler.
pub fn add_task(task: Arc<Task>) {
    GLOBAL_SCHEDULER
        .lock_irq_disabled()
        .as_mut()
        .unwrap()
        .enqueue(task);
}
