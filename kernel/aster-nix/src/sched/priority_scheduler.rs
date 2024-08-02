// SPDX-License-Identifier: MPL-2.0

use intrusive_collections::LinkedList;
use ostd::task::{set_scheduler, Scheduler, SharedTaskInfo, Task, TaskAdapter};

use crate::prelude::*;

pub fn init() {
    let preempt_scheduler = Box::new(PreemptScheduler::new());
    let scheduler = Box::<PreemptScheduler>::leak(preempt_scheduler);
    set_scheduler(scheduler);
}

/// The preempt scheduler
///
/// Real-time tasks are placed in the `real_time_tasks` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_tasks` queue and are only
/// scheduled for execution when there are no real-time tasks.
struct PreemptScheduler {
    /// Tasks with a priority of less than 100 are regarded as real-time tasks.
    real_time_tasks: SpinLock<LinkedList<TaskAdapter>>,
    /// Tasks with a priority greater than or equal to 100 are regarded as normal tasks.
    normal_tasks: SpinLock<LinkedList<TaskAdapter>>,
}

impl PreemptScheduler {
    pub fn new() -> Self {
        Self {
            real_time_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            normal_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
        }
    }
}

impl Scheduler for PreemptScheduler {
    fn enqueue(&self, task: Arc<Task>) {
        if task.priority().is_real_time() {
            self.real_time_tasks.lock_irq_disabled().push_back(task);
        } else {
            self.normal_tasks.lock_irq_disabled().push_back(task);
        }
    }

    fn dequeue(&self) -> Option<Arc<Task>> {
        if !self.real_time_tasks.lock_irq_disabled().is_empty() {
            self.real_time_tasks.lock_irq_disabled().pop_front()
        } else {
            self.normal_tasks.lock_irq_disabled().pop_front()
        }
    }

    fn should_preempt(&self, task: &SharedTaskInfo) -> bool {
        !task.priority.is_real_time() && !self.real_time_tasks.lock_irq_disabled().is_empty()
    }
}
