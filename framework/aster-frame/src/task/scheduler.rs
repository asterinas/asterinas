use crate::prelude::*;
use crate::sync::{SpinLock, SpinLockGuard};
use crate::task::{NeedResched, ReadPriority, Task};

use log::debug;

pub(crate) static GLOBAL_SCHEDULER: spin::Once<SpinLock<GlobalScheduler>> = spin::Once::new();

pub fn init() {
    GLOBAL_SCHEDULER.call_once(|| SpinLock::new(GlobalScheduler { scheduler: None }));
}

/// A scheduler for tasks.
///
/// Operations on the scheduler should be performed with interrupts disabled,
/// which has been ensured by the callers of the `GLOBAL_SCHEDULER`.
/// Therefore, implementations of this trait do not need to worry about interrupt safety.
pub trait Scheduler<T: NeedResched + ReadPriority = Task>: Sync + Send {
    /// Called when a task enters a runnable state.
    /// Add the task to the scheduler.
    fn enqueue(&self, task: Arc<T>);

    /// Called when the task is no longer alive.
    /// Remove the task-related from the scheduler.
    ///
    /// A similar method in the linux scheduler is `dequeue_task`.
    ///
    /// Return `true` if the task was in the scheduler.
    fn remove(&self, task: &Arc<T>) -> bool;

    /// Choose the most appropriate task eligible to run next.
    fn pick_next_task(&self) -> Option<Arc<T>>;

    /// Tells whether the given current task should be preempted
    /// by other tasks in the queue.
    fn should_preempt(&self, cur_task: &Arc<T>) -> bool;

    /// Handle a tick from the timer.
    /// Modify the states of the given task(the current task in `PROCESSOR`)
    /// according to the time update.
    ///
    /// # Arguments
    ///
    /// * `cur_task` - the task to be charged, must be held by the processor, and not in runqueue
    ///
    /// # Returns
    ///
    /// `true` if the `task` need a rescudule.
    fn tick(&self, cur_task: &Arc<T>) -> bool;

    /// Modify states before yielding the current task.
    /// Set the `need_resched` flag of the current task.
    fn before_yield(&self, cur_task: &Arc<T>) {
        cur_task.set_need_resched();
        debug!("task yielded");
    }

    /// Yield the current task to the target task at best effort.
    fn yield_to(&self, cur_task: &Arc<T>, target_task: Arc<T>);
}

pub struct GlobalScheduler {
    scheduler: Option<&'static dyn Scheduler>,
    // todo: multiple scheduler management
}

impl GlobalScheduler {
    pub fn new() -> Self {
        Self { scheduler: None }
    }

    /// fetch the next task to run from scheduler
    /// require the scheduler is not none
    pub fn fetch_next(&mut self) -> Option<Arc<Task>> {
        self.scheduler.unwrap().pick_next_task()
    }

    /// enqueue a task using scheduler
    /// require the scheduler is not none
    pub fn enqueue(&mut self, task: Arc<Task>) {
        self.scheduler.unwrap().enqueue(task)
    }

    /// Remove the task and its related information from the scheduler.
    pub fn remove(&mut self, task: &Arc<Task>) {
        self.scheduler.unwrap().remove(task);
    }

    pub fn should_preempt(&self, task: &Arc<Task>) -> bool {
        self.scheduler.unwrap().should_preempt(task)
    }

    pub fn tick(&self, task: &Arc<Task>) -> bool {
        self.scheduler.unwrap().tick(task)
    }

    pub fn before_yield(&self, task: &Arc<Task>) {
        self.scheduler.unwrap().before_yield(task)
    }

    pub fn yield_to(&self, cur_task: &Arc<Task>, target_task: Arc<Task>) {
        self.scheduler.unwrap().yield_to(cur_task, target_task)
    }
}

/// Set the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: &'static dyn Scheduler) {
    GLOBAL_SCHEDULER
        .get()
        .unwrap()
        .lock_irq_disabled()
        .scheduler = Some(scheduler);
}

/// Get the locked global task scheduler.
pub(super) fn locked_global_scheduler<'a>() -> SpinLockGuard<'a, GlobalScheduler> {
    GLOBAL_SCHEDULER.get().unwrap().lock_irq_disabled()
}

/// Fetch the next task to run from scheduler.
/// The scheduler will pick the most appropriate task eligible to run next if any.
pub fn fetch_next_task() -> Option<Arc<Task>> {
    let task = locked_global_scheduler().fetch_next();
    debug!("fetch next task: {:#X}", {
        if let Some(task) = &task {
            Arc::as_ptr(task) as usize
        } else {
            0
        }
    });
    task
}

/// Enqueue a task into scheduler.
pub fn add_task(task: Arc<Task>) {
    locked_global_scheduler().enqueue(task.clone());
    debug!("add task: {:#X}", Arc::as_ptr(&task) as usize);
}

/// Remove the task and all its related information from the scheduler.
pub fn remove_task(task: &Arc<Task>) {
    locked_global_scheduler().remove(task);
    debug!("remove task: {:#X}", Arc::as_ptr(task) as usize);
}
