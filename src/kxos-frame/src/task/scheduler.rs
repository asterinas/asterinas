use crate::prelude::*;
use crate::task::Task;

/// A scheduler for tasks.
///
/// An implementation of scheduler can attach scheduler-related information
/// with the `TypeMap` returned from `task.data()`.
pub trait Scheduler {
    fn enqueue(&self, task: Arc<Task>);

    fn dequeue(&self) -> Option<Arc<Task>>;
}

/// Set the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: &'static dyn Scheduler) {
    todo!()
}
