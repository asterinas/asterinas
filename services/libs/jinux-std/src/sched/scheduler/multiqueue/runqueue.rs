use super::prio_arr::PriorityArray;
use crate::prelude::*;
use jinux_frame::task::{Priority, Task};

/// discard the real-time task priority to obtain its index in the runqueue
// #[inline]
// fn prio_to_idx(prio: &Priority) -> usize {
//     debug_assert!(prio.get() >= REAL_TIME_TASK_PRI);
//     (prio.get() - REAL_TIME_TASK_PRI) as usize
// }

// /// Mainly used to record the clock-related statistics of a task
// #[derive(Copy, Clone, Default)]
// pub struct Statistics {
//     avg_sleep_time: u64,

// }

// impl Statistics {
//     pub fn update_avg_sleep_time(&mut self, sleep_time: u64) {
//         todo!();
//     }
// }

// pub struct SchedEntity {
//     pub task: Arc<Task>, // or a weak ref?
//     pub dyn_prio: Priority,
//     stat: Statistics,
// }

// impl SchedEntity {
//     pub fn new(task: Arc<Task>) -> Self {
//         Self {
//             task,
//             dyn_prio: task.priority(), // todo: don't expose the static priority
//             stat: Statistics::default(),
//         }
//     }

//     pub fn update_avg_sleep_time(&mut self, sleep_time: u64) {
//         self.stat.update_avg_sleep_time(sleep_time);
//     }

//     pub fn update_dyn_prio(&mut self) {
//         todo!();
//     }
// }

// impl From<Task> for SchedEntity {
//     fn from(task: Task) -> Self {
//         Self::new(task)
//     }
// }

pub struct RunQueue {
    active: PriorityArray,
    expired: PriorityArray,
    pub(crate) best_expired_prio: Priority,
    // most_recent_timestamp: u64,
    /// The ticks of the first task added into the expired arrays.
    /// For stavation detection.
    first_expired_timestamp: u64,
}

impl Default for RunQueue {
    fn default() -> Self {
        Self {
            active: PriorityArray::default(),
            expired: PriorityArray::default(),
            best_expired_prio: Priority::lowest(),
            // most_recent_timestamp: 0,
            first_expired_timestamp: 0,
        }
    }
}

impl RunQueue {
    fn swap_to_refill_active(&mut self) {
        debug_assert!(self.active.empty());
        core::mem::swap(&mut self.active, &mut self.expired);
    }

    pub fn activate(&mut self, task: Arc<Task>) {
        task.set_active(true);
        self.active.enqueue_task(task);
    }

    /// pick the next task to run from the active queues
    pub fn next_task(&mut self) -> Option<Arc<Task>> {
        if self.active.empty() && !self.expired.empty() {
            self.swap_to_refill_active();
        }
        // todo: update the statistics
        self.active.next_task()
    }

    pub fn expired_starving(&self) -> bool {
        todo!("expired_starving detection");
    }

    pub fn expire(&mut self, task: Arc<Task>, cur_tick: u64) {
        self._expire(task);
        if self.first_expired_timestamp == 0 {
            self.first_expired_timestamp = cur_tick;
        }
    }

    fn _expire(&mut self, task: Arc<Task>) {
        debug_assert!(self.active.empty() || !self.active.dequeue_task(&task));
        self.expired.enqueue_task(task.clone());
        task.set_active(false);
    }

    /// For better interactive performance,
    /// requeue the eq-priority tasks within the active array.
    pub fn roundrobin_requeue(&mut self, task: &Arc<Task>) {
        todo!()
    }
}
