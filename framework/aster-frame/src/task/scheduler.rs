// SPDX-License-Identifier: MPL-2.0

use lazy_static::lazy_static;

use crate::{cpu::this_cpu, cpu_local, prelude::*, sync::SpinLock, task::Task};

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
    /// Add a task to the scheduler's run queue.
    fn enqueue(&self, task: Arc<Task>);

    /// Remove and return a task from the scheduler's run queue for the given CPU.
    fn dequeue(&self, cpu_id: u32) -> Option<Arc<Task>>;

    /// Fetch a high priority task for potential preemption.
    fn preempt(&self, cpu_id: u32) -> Option<Arc<Task>>;
}

/// `GeneralScheduler` is a simple wrapper around a Scheduler trait object.
struct GeneralScheduler {
    scheduler: Option<&'static dyn Scheduler>,
}

impl GeneralScheduler {
    const fn new() -> Self {
        Self { scheduler: None }
    }

    /// Dequeues a task from the associated scheduler for the given CPU.
    /// It requires the scheduler to be set (not None).
    fn dequeue(&mut self, cpu_id: u32) -> Option<Arc<Task>> {
        self.scheduler.unwrap().dequeue(cpu_id)
    }

    /// Enqueues a task using the associated scheduler.
    /// It requires the scheduler to be set (not None).
    fn enqueue(&mut self, task: Arc<Task>) {
        self.scheduler.unwrap().enqueue(task)
    }

    /// Fetches a high-priority task for preemption based on the given CPU ID.
    /// It requires the scheduler to be set (not None).
    fn preempt(&mut self, cpu_id: u32) -> Option<Arc<Task>> {
        self.scheduler.unwrap().preempt(cpu_id)
    }
}

/// Initializes the global task scheduler.
///
/// This function sets the scheduler that will be used globally across CPUs.
/// It must be called before spawning any tasks to ensure they have a scheduler to manage them.
pub fn set_global_scheduler(scheduler: &'static dyn Scheduler) {
    GLOBAL_SCHEDULER.lock_irq_disabled().scheduler = Some(scheduler);
}

/// Retrieves a task from the global scheduler queue for the specified CPU.
pub fn fetch_task_from_global(cpu_id: u32) -> Option<Arc<Task>> {
    GLOBAL_SCHEDULER.lock_irq_disabled().dequeue(cpu_id)
}

/// Adds a new task to the global scheduler queue.
pub fn add_task_to_global(task: Arc<Task>) {
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
