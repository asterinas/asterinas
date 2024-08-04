// SPDX-License-Identifier: MPL-2.0

use intrusive_collections::LinkedList;
use ostd::task::{set_scheduler, Scheduler, Task, TaskAdapter};

use crate::prelude::*;

pub fn init() {
    set_scheduler(Box::new(PreemptScheduler::new()));
}

/// The preempt scheduler
///
/// Real-time tasks are placed in the `real_time_tasks` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_tasks` queue and are only
/// scheduled for execution when there are no real-time tasks.
struct PreemptScheduler {
    /// Tasks with a priority of less than 100 are regarded as real-time tasks.
    real_time_tasks: LinkedList<TaskAdapter>,
    /// Tasks with a priority greater than or equal to 100 are regarded as normal tasks.
    normal_tasks: LinkedList<TaskAdapter>,
}

impl PreemptScheduler {
    pub fn new() -> Self {
        Self {
            real_time_tasks: LinkedList::new(TaskAdapter::new()),
            normal_tasks: LinkedList::new(TaskAdapter::new()),
        }
    }
}

impl Scheduler for PreemptScheduler {
    fn enqueue(&mut self, task: Arc<Task>) {
        if task.is_real_time() {
            self.real_time_tasks.push_back(task);
        } else {
            self.normal_tasks.push_back(task);
        }
    }

    fn dequeue(&mut self) -> Option<Arc<Task>> {
        if !self.real_time_tasks.is_empty() {
            self.real_time_tasks.pop_front()
        } else {
            self.normal_tasks.pop_front()
        }
    }

    fn should_preempt(&mut self, task: &Arc<Task>) -> bool {
        !task.is_real_time() && !self.real_time_tasks.is_empty()
    }
}
