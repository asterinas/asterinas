use crate::prelude::*;
use crate::task::Task;

use lazy_static::lazy_static;
use spin::Mutex;

lazy_static! {
    pub(crate) static ref GLOBAL_SCHEDULER: Mutex<GlobalScheduler> =
        Mutex::new(GlobalScheduler { scheduler: None });
}

/// A scheduler for tasks.
///
/// An implementation of scheduler can attach scheduler-related information
/// with the `TypeMap` returned from `task.data()`.
pub trait Scheduler: Sync + Send {
    fn enqueue(&self, task: Arc<Task>);

    fn dequeue(&self) -> Option<Arc<Task>>;
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
}
/// Set the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: &'static dyn Scheduler) {
    GLOBAL_SCHEDULER.lock().scheduler = Some(scheduler);
}

pub fn fetch_task() -> Option<Arc<Task>> {
    GLOBAL_SCHEDULER.lock().dequeue()
}

pub fn add_task(task: Arc<Task>) {
    GLOBAL_SCHEDULER.lock().enqueue(task);
}
