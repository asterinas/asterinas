// SPDX-License-Identifier: MPL-2.0

use crate::config::SCHED_DEBUG_LOG;
use crate::prelude::*;
use crate::sync::{SpinLock, SpinLockGuard};
use crate::task::{NeedResched, ReadPriority, Task};

use log::debug;

pub(crate) static GLOBAL_SCHEDULER: spin::Once<SpinLock<GlobalScheduler>> = spin::Once::new();

pub fn init() {
    GLOBAL_SCHEDULER.call_once(|| SpinLock::new(GlobalScheduler { scheduler: None }));
}

/// The number of `Task`s.
pub type TaskNumber = u32;

/// A scheduler for tasks.
///
/// Operations on the scheduler should be performed with interrupts disabled,
/// which has been ensured by the callers of the `GLOBAL_SCHEDULER`.
/// Therefore, implementations of this trait do not need to worry about interrupt safety.
pub trait Scheduler<T: NeedResched + ReadPriority = Task>: Sync + Send {
    /// Called when a task enters a runnable state.
    /// Add the task to the scheduler.
    fn enqueue(&self, task: Arc<T>);

    /// Pick the task out of the scheduler if it is in the scheduler.
    /// Return `true` if the task was in the scheduler.
    fn dequeue(&self, task: &Arc<T>) -> bool;

    // FIXME: use purge/remove_dead instead?
    /// Called when the task is no longer alive.
    /// Remove all the task-related from the scheduler.
    /// Return `true` if the task was in the scheduler.
    fn remove(&self, task: &Arc<T>) -> bool {
        self.dequeue(task) && {
            self.clear(task);
            true
        }
    }

    /// Clear the all states of the given task from the scheduler.
    fn clear(&self, task: &Arc<T>) {}

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
    fn tick(&self, cur_task: &Arc<T>);

    /// Modify states before yielding the current task.
    /// Set the `need_resched` flag of the current task.
    fn before_yield(&self, cur_task: &Arc<T>) {
        cur_task.set_need_resched();
        if SCHED_DEBUG_LOG {
            debug!("before yield: {:#X}", Arc::as_ptr(cur_task) as usize);
        }
    }

    /// Yield the current task to the target task at best effort.
    fn yield_to(&self, cur_task: &Arc<T>, target_task: Arc<T>);

    fn contains(&self, task: &Arc<T>) -> bool;

    fn task_num(&self) -> TaskNumber;
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
    pub fn fetch_next(&self) -> Option<Arc<Task>> {
        self.scheduler.unwrap().pick_next_task()
    }

    /// enqueue a task using scheduler
    /// require the scheduler is not none
    pub fn enqueue(&self, task: Arc<Task>) {
        self.scheduler.unwrap().enqueue(task)
    }

    /// dequeue a task from the scheduler
    pub fn dequeue(&self, task: &Arc<Task>) -> bool {
        self.scheduler.unwrap().dequeue(task)
    }

    // FIXME: use purge/remove_dead instead?
    /// Remove the task and its related information from the scheduler.
    pub fn remove(&mut self, task: &Arc<Task>) {
        self.scheduler.unwrap().remove(task);
    }

    pub fn should_preempt(&self, task: &Arc<Task>) -> bool {
        self.scheduler.unwrap().should_preempt(task)
    }

    pub fn tick(&self, task: &Arc<Task>) {
        self.scheduler.unwrap().tick(task);
    }

    pub fn before_yield(&self, task: &Arc<Task>) {
        self.scheduler.unwrap().before_yield(task)
    }

    pub fn yield_to(&self, cur_task: &Arc<Task>, target_task: Arc<Task>) {
        self.scheduler.unwrap().yield_to(cur_task, target_task)
    }

    pub fn contains(&self, task: &Arc<Task>) -> bool {
        self.scheduler.unwrap().contains(task)
    }

    pub fn task_num(&self) -> TaskNumber {
        self.scheduler.unwrap().task_num()
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
    if SCHED_DEBUG_LOG {
        debug!("fetch next task: {:#X}", {
            if let Some(task) = &task {
                Arc::as_ptr(task) as usize
            } else {
                0
            }
        });
    }
    task
}

/// Enqueue a task into scheduler.
pub fn add_task(task: Arc<Task>) {
    locked_global_scheduler().enqueue(task.clone());
    if SCHED_DEBUG_LOG {
        debug!("add task: {:#X}", Arc::as_ptr(&task) as usize);
    }
}

/// Remove the task and all its related information from the scheduler.
pub fn remove_task(task: &Arc<Task>) {
    locked_global_scheduler().remove(task);
    if SCHED_DEBUG_LOG {
        debug!("remove task: {:#X}", Arc::as_ptr(task) as usize);
    }
}

/// Whether the given task has been queued in the scheduler.
pub fn task_queued(task: &Arc<Task>) -> bool {
    locked_global_scheduler().contains(task)
}
