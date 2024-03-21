// SPDX-License-Identifier: MPL-2.0

use lazy_static::lazy_static;

use crate::{prelude::*, sync::SpinLock, task::Task};

lazy_static! {
    /// The global scheduler is responsible for managing tasks across all CPUs in the system.
    /// It provides a centralized point of task management, and all tasks will be put into the
    /// queue of the global scheduler first when they are ready to run. The global scheduler can decide
    /// the distribution and migration of tasks according to the CPU load to achieve the best resource utilization.
    static ref GLOBAL_SCHEDULER: SpinLock<GeneralScheduler> =
        SpinLock::new(GeneralScheduler::new());
}

cpu_local! {
    /// Each CPU in the system has its own local scheduler instance, allowing for tasks to be
    /// managed on a per-CPU basis. This is useful for tasks that are affine to a specific CPU
    /// or for load balancing purposes where tasks are distributed among CPUs to avoid contention.
    /// The local scheduler can optimize task execution by reducing task migration between CPUs and
    /// minimizing synchronization overhead for task management.Furthermore, If a preempted task
    /// is emplaced back immediately to the global scheduler before the context switch, other processors
    /// may fetch the task with a stale context. And we couldn't make the progress between emplacing
    /// and context storing atomic.
    static LOCAL_SCHEDULER: GeneralScheduler = GeneralScheduler::new();
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

/// This function is used to fetch a high-priority task from the global scheduler,
/// potentially to replace a currently running task if it is not high priority.
pub fn preempt_global(cpu_id: u32) -> Option<Arc<Task>> {
    GLOBAL_SCHEDULER.lock_irq_disabled().preempt(cpu_id)
}

/// Sets the local scheduler for the current CPU.
pub fn set_local_scheduler(scheduler: &'static dyn Scheduler) {
    LOCAL_SCHEDULER.borrow().scheduler = Some(scheduler);
}

/// Sets the local scheduler for the current CPU.
pub fn fetch_task_from_local() -> Option<Arc<Task>> {
    LOCAL_SCHEDULER.borrow().dequeue(this_cpu())
}

/// Fetches a task from the local scheduler queue of the current CPU.
pub fn add_task_to_local(task: Arc<Task>) {
    LOCAL_SCHEDULER.borrow().enqueue(task);
}

pub fn add_sleeping_task_to_local(task: Arc<Task>) {
    LOCAL_SCHEDULER.borrow().enqueue_sleep(task);
}

/// This function is used to check for and retrieve a high-priority task from the local scheduler,
/// which could preempt the currently running task on this CPU if necessary.
pub fn preempt_local() -> Option<Arc<Task>> {
    LOCAL_SCHEDULER.borrow().preempt(this_cpu())
}
