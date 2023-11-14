use crate::prelude::*;
use intrusive_collections::LinkedList;
use jinux_frame::task::{Scheduler, Task, TaskAdapter};

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
}

impl Scheduler for PreemptiveFIFOScheduler {
    fn activate(&self, task: Arc<Task>) {
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

    fn pick_next_task(&self) -> Option<Arc<Task>> {
        if !self.real_time_tasks.lock_irq_disabled().is_empty() {
            self.real_time_tasks.lock_irq_disabled().pop_front()
        } else {
            self.normal_tasks.lock_irq_disabled().pop_front()
        }
    }

    fn should_preempt(&self, task: &Arc<Task>) -> bool {
        task.need_resched()
            || !task.is_real_time() && !self.real_time_tasks.lock_irq_disabled().is_empty()
    }

    fn tick(&self, task: &Arc<Task>, cur_tick: u64) -> bool {
        task.need_resched()
    }
}
