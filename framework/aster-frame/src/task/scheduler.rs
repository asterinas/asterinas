// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    sync::{SpinLock, SpinLockGuard},
    task::{SchedTaskBase, Task},
};

pub(crate) static GLOBAL_SCHEDULER: spin::Once<SpinLock<GlobalScheduler>> = spin::Once::new();

pub fn init() {
    GLOBAL_SCHEDULER.call_once(|| SpinLock::new(GlobalScheduler { scheduler: None }));
}

/// Logs the scheduler debug information.
///
/// Turn on the `SCHED_DEBUG_LOG` in `config.rs` to enable this macro.
/// The logs will be outputted at the [`Debug`] level.
///
/// [`Debug`]: log::Level::Debug
///
/// # Examples
///
/// ```
/// sched_debug!("{} debug information", 1);
/// ```
///
#[macro_export]
macro_rules! sched_debug {
    () => {};
    ($fmt: literal $(, $($arg: tt)+)?) => {
        if $crate::config::SCHED_DEBUG_LOG {
            log::debug!($fmt $(, $($arg)+)?);
        }
    }
}

/// A scheduler for tasks.
///
/// Operations on the scheduler should be performed with interrupts disabled,
/// which has been ensured by the callers of the `GLOBAL_SCHEDULER`.
/// Therefore, implementations of this trait do not need to worry about interrupt safety.
pub trait Scheduler<T: SchedTaskBase = Task>: Sync + Send {
    /// Add the task to the scheduler when it enters a runnable state.
    fn enqueue(&self, task: Arc<T>);

    /// Pick the most appropriate task eligible to run next from the scheduler.
    fn pick_next_task(&self) -> Option<Arc<T>>;

    /// Remove the task-related from the scheduler when the task is no longer alive.
    fn clear(&self, task: &Arc<T>);

    /// Tells whether the current task should be preempted by tasks in the queue.
    ///
    /// # Panics
    ///
    /// Panics if the current task is none.
    fn should_preempt_cur_task(&self) -> bool;

    /// Handle a tick from the timer.
    /// Modify the states of the current task on the processor
    /// according to the time update.
    ///
    /// # Panics
    ///
    /// Panics if the current task is none.
    fn tick_cur_task(&self);

    /// Modify states before yielding the current task.
    /// Set the `need_resched` flag of the current task.
    ///
    /// # Panics
    ///
    /// Panics if the current task is none.
    fn prepare_to_yield_cur_task(&self) {
        let cur_task = T::current();
        cur_task.set_need_resched(true);
        sched_debug!("before yield: {:#X}", Arc::as_ptr(&cur_task) as usize);
    }

    // FIXME: remove this after merging #632.
    /// Yield the current task to the given task at best effort.
    fn prepare_to_yield_to(&self, task: Arc<T>);
}

pub struct GlobalScheduler {
    scheduler: Option<&'static dyn Scheduler>,
    // TODO: add multiple scheduler management
}

impl GlobalScheduler {
    pub fn new() -> Self {
        Self { scheduler: None }
    }

    /// Pick the next task to run from scheduler.
    /// Require the scheduler is not none.
    pub fn pick_next_task(&self) -> Option<Arc<Task>> {
        self.scheduler.unwrap().pick_next_task()
    }

    /// Enqueue a task into scheduler.
    /// Require the scheduler is not none.
    pub fn enqueue(&self, task: Arc<Task>) {
        self.scheduler.unwrap().enqueue(task)
    }

    /// Remove the task and its related information from the scheduler.
    pub fn clear(&self, task: &Arc<Task>) {
        self.scheduler.unwrap().clear(task);
    }

    pub fn should_preempt_cur_task(&self) -> bool {
        self.scheduler.unwrap().should_preempt_cur_task()
    }

    pub fn tick_cur_task(&self) {
        self.scheduler.unwrap().tick_cur_task();
    }

    pub fn prepare_to_yield_cur_task(&self) {
        self.scheduler.unwrap().prepare_to_yield_cur_task()
    }

    // FIXME: remove this after merging #632.
    pub fn prepare_to_yield_to(&self, target_task: Arc<Task>) {
        self.scheduler.unwrap().prepare_to_yield_to(target_task)
    }
}

/// Set the global task scheduler.
///
/// This must be called before invoking `Task::spawn`.
pub fn set_scheduler(scheduler: &'static dyn Scheduler) {
    locked_global_scheduler().scheduler = Some(scheduler);
}

/// Get the locked global task scheduler.
pub(super) fn locked_global_scheduler<'a>() -> SpinLockGuard<'a, GlobalScheduler> {
    GLOBAL_SCHEDULER.get().unwrap().lock_irq_disabled()
}

/// Pick the next task to run from scheduler.
/// The scheduler will pick the most appropriate task eligible to run next if any.
pub fn pick_next_task() -> Option<Arc<Task>> {
    let task = locked_global_scheduler().pick_next_task();
    sched_debug!(
        "fetch next task: {:#X}",
        task.as_ref().map(|t| Arc::as_ptr(t) as usize).unwrap_or(0)
    );
    task
}

/// Enqueue a task into scheduler.
pub fn add_task(task: Arc<Task>) {
    locked_global_scheduler().enqueue(task.clone());
    sched_debug!("add task: {:#X}", Arc::as_ptr(&task) as usize);
}

/// Remove all the information of the task from the scheduler.
pub fn clear_task(task: &Arc<Task>) {
    locked_global_scheduler().clear(task);
    sched_debug!("remove task: {:#X}", Arc::as_ptr(task) as usize);
}
