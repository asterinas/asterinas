use crate::prelude::*;
use bitmaps::Bitmap;
use intrusive_collections::LinkedList;
use jinux_frame::task::{Priority, Task, TaskAdapter};

const MIN_PRIORITY: usize = Priority::highest().get() as usize;
const MAX_PRIORITY: usize = Priority::lowest().get() as usize;
const NUM_PRIORITY: usize = MAX_PRIORITY - MIN_PRIORITY + 1;

/// Task queues indexed by numeric priorities
pub struct PriorityArray {
    /// total number of tasks in the queue
    nr_active: usize,

    /// array of task queues, one per priority, indexed by priority
    queue: Vec<LinkedList<TaskAdapter>>,

    /// bitmap of non-empty queues
    bitmap: Bitmap<NUM_PRIORITY>,
}

impl PartialEq for PriorityArray {
    fn eq(&self, other: &Self) -> bool {
        core::ptr::eq(self, other)
    }
}
impl Eq for PriorityArray {}

impl Default for PriorityArray {
    fn default() -> Self {
        let queue: Vec<_> = (MIN_PRIORITY..(MAX_PRIORITY + 1))
            .map(|_| LinkedList::new(TaskAdapter::new()))
            .collect();
        debug_assert!(queue.len() == NUM_PRIORITY);
        Self {
            nr_active: 0,
            queue,
            bitmap: Bitmap::new(),
        }
    }
}

impl PriorityArray {
    #[inline]
    pub fn empty(&self) -> bool {
        self.nr_active == 0
    }

    /// Adding a task to this priority array
    pub fn enqueue_task(&mut self, task: Arc<Task>) {
        self.bitmap.set(task.dyn_prio().get() as usize, true);
        self.inc_nr_active();
        self.queue[task.dyn_prio().get() as usize].push_back(task);
    }

    #[inline]
    fn inc_nr_active(&mut self) {
        self.nr_active.checked_add(1).expect("task number overflow");
    }

    #[inline]
    fn dec_nr_active(&mut self) {
        // debug_assert!(self.nr_active > 0);
        self.nr_active -= 1;
    }

    /// Removing a task from this priority array
    /// Returns true if the task is found and removed, false otherwise
    pub fn dequeue_task(&mut self, task: &Arc<Task>) -> bool {
        self.dec_nr_active();
        let idx = task.dyn_prio().get() as usize;
        let found = self.rm_task_from_queue(task);
        if found && self.queue[idx].is_empty() {
            self.bitmap.set(idx, false);
        }
        found
    }

    /// Remove a task from the queue
    /// Returns true if the task is found and removed, false otherwise
    fn rm_task_from_queue(&mut self, task: &Arc<Task>) -> bool {
        let idx = task.dyn_prio().get() as usize;
        let mut cursor = self.queue[idx].cursor_mut();
        while let Some(t) = cursor.get() {
            if t == task.as_ref() {
                return cursor.remove().is_some();
            }
            cursor.move_next();
        }
        debug_assert!(cursor.is_null());
        false // not found
    }

    /// Put task to the end of the run list without the overhead of dequeue
    /// followed by enqueue.
    pub fn requeue(&mut self, task: &Arc<Task>) -> bool {
        let found = self.rm_task_from_queue(task);
        if found {
            let idx = task.dyn_prio().get() as usize;
            self.queue[idx].push_back(task.clone());
        }
        found
    }

    /// Pick the next task to run from the active queues in a Round-Robin manner
    pub fn next_task(&mut self) -> Option<Arc<Task>> {
        match self.bitmap.first_index() {
            Some(idx) => {
                let next_task = self.queue[idx].pop_front();
                debug_assert!(next_task.is_some());
                self.dec_nr_active();
                if self.queue[idx].is_empty() {
                    self.bitmap.set(idx, false);
                }
                next_task
            }
            None => None,
        }
    }
}
