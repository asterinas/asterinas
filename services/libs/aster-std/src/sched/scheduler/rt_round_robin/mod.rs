// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::{AtomicU32, Ordering::Relaxed};

use crate::prelude::*;
use aster_frame::task::{NeedResched, ReadPriority, Scheduler, Task, TaskAdapter, TaskNumber};
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
    /// The total number of tasks in this scheduler.
    task_num: AtomicU32,
}

impl PreemptiveRRScheduler {
    pub fn new() -> Self {
        Self {
            real_time_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            normal_tasks: SpinLock::new(LinkedList::new(TaskAdapter::new())),
            task_num: AtomicU32::new(0),
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
        let mut target = if task.is_real_time() {
            self.real_time_tasks.lock()
        } else {
            self.normal_tasks.lock()
        };
        if front {
            target.push_front(task);
        } else {
            target.push_back(task);
        }
        self.task_num.fetch_add(1, Relaxed);
    }
}

impl Scheduler for PreemptiveRRScheduler {
    fn enqueue(&self, task: Arc<Task>) {
        self.enqueue_at(task, false);
    }

    fn dequeue(&self, task: &Arc<Task>) -> bool {
        let mut target = if task.is_real_time() {
            self.real_time_tasks.lock()
        } else {
            self.normal_tasks.lock()
        };
        let found = Self::rm_task_from_queue(task, target.cursor_mut());
        if found {
            self.task_num.fetch_sub(1, Relaxed);
        }
        found
    }

    fn pick_next_task(&self) -> Option<Arc<Task>> {
        let picked = if !self.real_time_tasks.lock().is_empty() {
            self.real_time_tasks.lock().pop_front()
        } else {
            self.normal_tasks.lock().pop_front()
        };
        if picked.is_some() {
            self.task_num.fetch_sub(1, Relaxed);
        }
        picked
    }

    fn should_preempt(&self, task: &Arc<Task>) -> bool {
        // task.need_resched()
        !task.is_real_time() && !self.real_time_tasks.lock().is_empty()
    }

    fn tick(&self, task: &Arc<Task>) {}

    fn yield_to(&self, cur_task: &Arc<Task>, target_task: Arc<Task>) {
        self.before_yield(cur_task);
        self.enqueue_at(target_task, true);
    }

    fn contains(&self, task: &Arc<Task>) -> bool {
        let target = &mut if task.is_real_time() {
            self.real_time_tasks.lock()
        } else {
            self.normal_tasks.lock()
        };

        let cursor = &mut target.cursor_mut();
        while let Some(t) = cursor.get() {
            if t == task.as_ref() {
                return true;
            }
            cursor.move_next();
        }
        false
    }

    fn task_num(&self) -> TaskNumber {
        self.task_num.load(Relaxed)
    }
}
