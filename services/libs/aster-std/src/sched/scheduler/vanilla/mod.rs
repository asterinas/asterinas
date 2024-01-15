use core::sync::atomic::AtomicU32;

use crate::prelude::*;
use aster_frame::task::{NeedResched, ReadPriority, Scheduler, Task, TaskAdapter};
use core::sync::atomic::Ordering::Relaxed;
use intrusive_collections::{linked_list::CursorMut, LinkedList};

/// The preempt scheduler
///
/// Real-time tasks are placed in the `real_time_tasks` queue and
/// are always prioritized during scheduling.
/// Normal tasks are placed in the `normal_tasks` queue and are only
/// scheduled for execution when there are no real-time tasks.
pub struct PreemptiveRRScheduler {
    /// Tasks with a priority of less than 100 are regarded as real-time tasks.
    real_time_tasks: SpinLock<LinkedList<TaskAdapter>>,
    /// Tasks with a priority greater than or equal to 100 are regarded as normal tasks.
    normal_tasks: SpinLock<LinkedList<TaskAdapter>>,
    /// The total number of tasks in the scheduler.
    num_tasks: AtomicU32,
}

impl PreemptiveRRScheduler {
    pub fn new() -> Self {
        Self {
            real_time_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            normal_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            num_tasks: 0.into(),
        }
    }

    /// Remove a task from the queue
    /// Returns true if the task is found and removed, false otherwise
    fn rm_task_from_queue(task: &Arc<Task>, mut cursor: CursorMut<'_, TaskAdapter>) -> bool {
        while let Some(t) = cursor.get() {
            if t == task.as_ref() {
                return cursor.remove().is_some();
            }
            cursor.move_next();
        }
        debug_assert!(cursor.is_null());
        false // not found
    }

    fn enqueue_at(&self, task: Arc<Task>, front: bool) {
        task.clear_need_resched();
        self.num_tasks.fetch_add(1, Relaxed);
        let mut target = if task.is_real_time() {
            self.real_time_tasks.lock_irq_disabled()
        } else {
            self.normal_tasks.lock_irq_disabled()
        };
        if front {
            target.push_front(task);
        } else {
            target.push_back(task);
        }
    }

    fn target_queue(&self, task: &Arc<Task>) -> SpinLockGuard<'_, LinkedList<TaskAdapter>> {
        if task.is_real_time() {
            self.real_time_tasks.lock_irq_disabled()
        } else {
            self.normal_tasks.lock_irq_disabled()
        }
    }
}

impl Scheduler for PreemptiveRRScheduler {
    fn enqueue(&self, task: Arc<Task>) {
        self.enqueue_at(task, false);
    }

    fn remove(&self, task: &Arc<Task>) -> bool {
        let found = Self::rm_task_from_queue(task, self.target_queue(task).cursor_mut());
        if found {
            self.num_tasks.fetch_sub(1, Relaxed);
        }
        found
    }

    fn pick_next_task(&self) -> Option<Arc<Task>> {
        self.num_tasks.fetch_sub(1, Relaxed);
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

    fn tick(&self, task: &Arc<Task>) {}

    fn yield_to(&self, cur_task: &Arc<Task>, target_task: Arc<Task>) {
        self.before_yield(cur_task);
        self.enqueue_at(target_task, true);
    }

    #[cfg(any(test, ktest))]
    fn task_num(&self) -> aster_frame::task::TaskNumber {
        self.num_tasks.load(Relaxed)
    }

    #[cfg(any(test, ktest))]
    fn contains(&self, task: &Arc<Task>) -> bool {
        let target = self.target_queue(task);
        let cursor = &mut target.cursor();
        while let Some(t) = cursor.get() {
            if t == task.as_ref() {
                return true;
            }
            cursor.move_next();
        }
        false
    }
}
