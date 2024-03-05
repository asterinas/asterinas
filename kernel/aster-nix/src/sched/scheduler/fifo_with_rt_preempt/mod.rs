// SPDX-License-Identifier: MPL-2.0

use aster_frame::task::{Current, NeedResched, ReadPriority, Scheduler, Task, TaskAdapter};
use intrusive_collections::LinkedList;

use crate::prelude::*;

/// The preempt scheduler
///
/// Real-time tasks are placed in the `real_time_tasks` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_tasks` queue and are only
/// scheduled for execution when there are no real-time tasks.
pub struct PreemptiveFIFOScheduler {
    /// Tasks with a priority of less than 100 are regarded as real-time tasks.
    real_time_tasks: SpinLock<LinkedList<TaskAdapter>>,
    /// Tasks with a priority greater than or equal to 100 are regarded as normal tasks.
    normal_tasks: SpinLock<LinkedList<TaskAdapter>>,
}

impl PreemptiveFIFOScheduler {
    pub fn new() -> Self {
        Self {
            real_time_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            normal_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
        }
    }

    fn find_queue<'a>(&'a self, task: &Arc<Task>) -> SpinLockGuard<'a, LinkedList<TaskAdapter>> {
        if task.is_real_time() {
            self.real_time_tasks.lock()
        } else {
            self.normal_tasks.lock()
        }
    }
}

impl Scheduler for PreemptiveFIFOScheduler {
    fn enqueue(&self, task: Arc<Task>) {
        task.set_need_resched(false);
        self.find_queue(&task).push_back(task);
    }

    fn clear(&self, task: &Arc<Task>) {}

    fn pick_next_task(&self) -> Option<Arc<Task>> {
        if !self.real_time_tasks.lock().is_empty() {
            self.real_time_tasks.lock().pop_front()
        } else {
            self.normal_tasks.lock().pop_front()
        }
    }

    fn should_preempt_cur_task(&self) -> bool {
        let task = Task::current();
        // task.need_resched()
        !task.is_real_time() && !self.real_time_tasks.lock().is_empty()
    }

    fn tick_cur_task(&self) {}

    fn prepare_to_yield_to(&self, task: Arc<Task>) {
        self.prepare_to_yield_cur_task();
        task.set_need_resched(false);
        self.find_queue(&task).push_front(task);
    }
}
