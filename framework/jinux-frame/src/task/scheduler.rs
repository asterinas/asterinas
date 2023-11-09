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
    fn activate(&self, task: Arc<Task>);

    fn fetch_next(&self) -> Option<Arc<Task>>;

    /// Tells whether the given task should be preempted by other tasks in the queue.
    fn should_preempt(&self, task: &Arc<Task>) -> bool;

    /// Charge a tick to the given task.
    ///
    /// # Arguments
    ///
    /// * `task` - the task to charge, must be held by the processor, and not in runqueue
    /// * `cur_tick` - the current tick
    fn tick(&self, task: &Arc<Task>, cur_tick: u64); // or mutable self ref?
}

pub struct GlobalScheduler {
    scheduler: Option<&'static dyn Scheduler>,
    // todo: multiple scheduler management
}

impl GlobalScheduler {
    pub fn new() -> Self {
        Self { scheduler: None }
    }

    /// dequeue a task using scheduler
    /// require the scheduler is not none
    pub fn fetch_next(&mut self) -> Option<Arc<Task>> {
        self.scheduler.unwrap().fetch_next()
    }
    /// enqueue a task using scheduler
    /// require the scheduler is not none
    pub fn enqueue(&mut self, task: Arc<Task>) {
        self.scheduler.unwrap().activate(task)
    }

    pub fn should_preempt(&self, task: &Arc<Task>) -> bool {
        self.scheduler.unwrap().should_preempt(task)
    }

    pub fn tick(&self, task: Arc<Task>, cur_tick: u64) {
        self.scheduler.unwrap().tick(&task, cur_tick);
    }
}

/// Set the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: &'static dyn Scheduler) {
    GLOBAL_SCHEDULER.lock_irq_disabled().scheduler = Some(scheduler);
}

pub fn fetch_task() -> Option<Arc<Task>> {
    GLOBAL_SCHEDULER.lock_irq_disabled().fetch_next()
}

pub fn add_task(task: Arc<Task>) {
    GLOBAL_SCHEDULER.lock_irq_disabled().enqueue(task);
    // todo: if the priority of the new task is higher than the current
    // task and the current task is preemptible, invoke `schedule()`.
}
