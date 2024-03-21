// SPDX-License-Identifier: MPL-2.0

//! At present, we use' PreemptGlobalScheduler' and' PreemptLocalScheduler' to meet the requirements of
//! priority preemption and multi-processor for scheduler. Currently, This is a simple solution.
//!
//! - For the requirement of priority preemption, we only distinguish between high-priority tasks
//! (real_time_tasks) and low-priority tasks (normal_tasks). Low-priority tasks will give up CPU at
//! the preemption point, if there are high-priority tasks at that time. Once the high-priority task
//! gets the CPU, it will be executed without being preempted by other high-priority tasks until the
//! task ends or it voluntarily gives up the CPU.
//!
//! - For the requirements of multi-core system, we divide the scheduler into local and global. At present,
//! it is a simple implementation, regardless of fairness and load balancing. All tasks are put into the
//! global scheduler when they are ready. The global scheduler will not actively distribute these tasks,
//! but rely on the local scheduler to actively request tasks for execution. Task switch and preemption
//! on on each CPU are the responsibility of the local scheduler. When the local scheduler is in task switch,
//! it will always actively request a task from the global scheduler, regardless of whether its own queue is
//! empty or not. This is to avoid some tasks stuck in the lazy global scheduler. When preempting, the local
//! scheduler will first check whether there are high-priority tasks in the local queue, and if not, it will
//! preempt the global scheduler. There may be a situation where each local scheduler has a high-priority task,
//! while there are also high-priority tasks in the global scheduler, and the local scheduler will not put the
//! global high-priority tasks when preempting. But don't worry that it will stay. When the local high-priority task
//! is in task switch, the local scheduler will take the initiative to get the global high-priority task from
//! the global scheduler. This is because local high-priority tasks will not be preempted, and it will not help
//! to put global high-priority tasks locally in advance.

use aster_frame::{
    cpu::this_cpu,
    task::{fetch_task_from_global, preempt_global, Scheduler, Task, TaskAdapter},
};
use intrusive_collections::LinkedList;

use crate::prelude::*;

/// The preempt global scheduler.
///
/// This scheduler is responsible for managing tasks at a global level, allowing for preemption
/// across all CPUs. It operates with two distinct queues: one for real-time tasks and another
/// for normal tasks. Real-time tasks are given higher priority in scheduling decisions, ensuring
/// that they are executed before normal tasks whenever possible.
pub(super) struct PreemptGlobalScheduler {
    /// Queue for real-time tasks, which have higher priority.
    /// The scheduler always checks this queue first for tasks to run, ensuring real-time tasks
    /// take precedence over normal tasks.
    real_time_tasks: SpinLock<LinkedList<TaskAdapter>>,
    /// Queue for normal tasks, which have lower priority.
    /// These tasks are scheduled for execution only when there are no real-time tasks pending.
    normal_tasks: SpinLock<LinkedList<TaskAdapter>>,
}

/// The preempt local scheduler.
///
/// Similar to the global scheduler, the local scheduler maintains separate queues for real-time
/// and normal tasks. However, the local scheduler operates at the CPU level, managing tasks that
/// are specific to a single CPU.
pub(super) struct PreemptLocalScheduler {
    /// Queue for real-time tasks specific to a CPU.
    real_time_tasks: SpinLock<LinkedList<TaskAdapter>>,
    /// Queue for normal tasks specific to a CPU.
    normal_tasks: SpinLock<LinkedList<TaskAdapter>>,
}

impl PreemptGlobalScheduler {
    pub fn new() -> Self {
        Self {
            real_time_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            normal_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
        }
    }
}

impl Scheduler for PreemptGlobalScheduler {
    fn enqueue(&self, task: Arc<Task>) {
        if task.is_real_time() {
            self.real_time_tasks
                .lock_irq_disabled()
                .push_back(task.clone());
        } else {
            self.normal_tasks
                .lock_irq_disabled()
                .push_back(task.clone());
        }
    }

    /// Dequeues a task from the scheduler based on CPU affinity and priority.
    ///
    /// This method first attempts to dequeue a real-time task that is affine to the given CPU.
    /// If no real-time tasks are affine or available, it then attempts to dequeue a normal task.
    fn dequeue(&self, cpu_id: u32) -> Option<Arc<Task>> {
        let mut real_time_tasks = self.real_time_tasks.lock_irq_disabled();
        let mut cursor = real_time_tasks.front_mut();
        while let Some(task_ref) = cursor.get() {
            if task_ref.cpu_affinity().contains(cpu_id) {
                return cursor.remove();
            } else {
                cursor.move_next();
            }
        }

        let mut normal_tasks = self.normal_tasks.lock_irq_disabled();
        let mut cursor = normal_tasks.front_mut();
        while let Some(task_ref) = cursor.get() {
            if task_ref.cpu_affinity().contains(cpu_id) {
                return cursor.remove();
            } else {
                cursor.move_next();
            }
        }
        None
    }

    /// This function is used to fetch a high-priority task from the global scheduler,
    /// potentially to replace a currently running task if it is not high priority.
    fn preempt(&self, cpu_id: u32) -> Option<Arc<Task>> {
        let mut real_time_tasks = self.real_time_tasks.lock_irq_disabled();
        let mut cursor = real_time_tasks.front_mut();
        while let Some(task_ref) = cursor.get() {
            if task_ref.cpu_affinity().contains(cpu_id) {
                return cursor.remove();
            } else {
                cursor.move_next();
            }
        }
        None
    }
}

impl PreemptLocalScheduler {
    pub fn new() -> Self {
        Self {
            real_time_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            normal_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
        }
    }
}

impl Scheduler for PreemptLocalScheduler {
    fn enqueue(&self, task: Arc<Task>) {
        if task.is_real_time() {
            self.real_time_tasks
                .lock_irq_disabled()
                .push_back(task.clone());
        } else {
            self.normal_tasks
                .lock_irq_disabled()
                .push_back(task.clone());
        }
    }

    /// Dequeues a task from the local scheduler queue.
    ///
    /// It always attempts to fetch a task from the global scheduler and enqueue it locally.
    /// Because the global scheduler does not actively distribute tasks, this provides a mechanism for
    /// pulling tasks into the local scheduler from the global queue.
    fn dequeue(&self, cpu_id: u32) -> Option<Arc<Task>> {
        assert!(cpu_id == this_cpu());
        if let Some(fetch_task) = fetch_task_from_global(cpu_id) {
            self.enqueue(fetch_task);
        }

        if let Some(task) = self.real_time_tasks.lock_irq_disabled().pop_front() {
            assert!(task.cpu_affinity().contains(cpu_id));
            return Some(task);
        }
        if let Some(task) = self.normal_tasks.lock_irq_disabled().pop_front() {
            assert!(task.cpu_affinity().contains(cpu_id));
            return Some(task);
        }
        None
    }

    /// Preempts the current task on the local CPU, if a higher-priority real-time task is available.
    ///
    /// It first attempts to preempt a real-time task from the local queue. If a real-time task is available
    /// and affine to the CPU, it is returned. If no real-time tasks are available locally, it
    /// attempts to preempt a task from the global scheduler.
    fn preempt(&self, cpu_id: u32) -> Option<Arc<Task>> {
        assert!(cpu_id == this_cpu());
        if let Some(task) = self.real_time_tasks.lock_irq_disabled().pop_front() {
            assert!(task.cpu_affinity().contains(cpu_id));
            return Some(task);
        }

        preempt_global(cpu_id)
    }
}
