mod dyn_prio;
mod interactive;
mod prio_arr;
pub mod runqueue;

use crate::prelude::*;
use core::sync::atomic::{AtomicUsize, Ordering};
use jinux_frame::task::{Scheduler, Task};
use runqueue::RunQueue;

pub struct MultiQueueScheduler {
    rq: SpinLock<RunQueue>,
    nr_running: AtomicUsize,
    // todo: into atomic
    // nr_switches: usize,
    // nr_sleeping: usize,
    // nr_uninterruptible
}

impl MultiQueueScheduler {
    pub fn new() -> Self {
        Self {
            rq: SpinLock::new(RunQueue::default()),
            nr_running: AtomicUsize::new(0),
        }
    }

    fn time_slice(&self, task: &Arc<Task>) -> u64 {
        todo!("calculate time slice")
    }

    fn tick_rt(&self, task: &Arc<Task>, cur_tick: u64) {
        debug_assert!(task.is_real_time());
        todo!("tick real-time task")
    }

    fn recharge_task(&self, task: &Arc<Task>) {
        if task.is_real_time() {
            // todo: recharge RoundRobin real-time tasks
            todo!("recharge real-time task");
        } else {
            task.set_dyn_prio(dyn_prio::effective_prio(task));
            task.set_time_slice(self.time_slice(task));
            task.deny_first_time_slice();
        }
    }

    fn tick_normal(&self, task: &Arc<Task>, cur_tick: u64) {
        debug_assert!(!task.is_real_time());
        let mut rq = self.rq.lock_irq_disabled();
        task.set_time_slice(task.time_slice() - 1);
        if task.time_slice() == 0 {
            task.set_need_resched();
            self.recharge_task(task);

            if interactive::is_interactive(task) && !rq.expired_starving() {
                rq.activate(task.clone());
                if task.priority() > rq.best_expired_prio {
                    rq.best_expired_prio = task.priority();
                }
            } else {
                rq.expire(task.clone(), cur_tick);
            }
        } else {
            rq.roundrobin_requeue(task);
        }
    }
}

/// Remove a task from the runqueue.
fn deactivate(task: Arc<Task>) {
    todo!();
    // line 979
}

impl Scheduler for MultiQueueScheduler {
    /// Move a task to the runqueue and do priority recalculation.
    /// Update all the scheduling statistics stuff.
    /// (sleep average calculation, priority modifiers, etc.)
    fn activate(&self, task: Arc<Task>) {
        // todo!("activate a task into runqueue");
        self.rq.lock_irq_disabled().activate(task);
        // around line 1039
        self.nr_running.fetch_add(1, Ordering::SeqCst);
    }

    fn fetch_next(&self) -> Option<Arc<Task>> {
        match self.rq.lock_irq_disabled().next_task() {
            Some(task) => {
                if task.need_resched() {
                    self.rq.lock_irq_disabled().expire_without_tick(task);
                    self.fetch_next()
                } else {
                    self.nr_running.fetch_sub(1, Ordering::SeqCst);
                    Some(task)
                }
            }
            None => None,
        }
    }

    fn should_preempt(&self, task: &Arc<Task>) -> bool {
        if task.need_resched() || !task.status().is_runnable() {
            return true;
        }
        todo!("if there is a higher priority task in the runqueue")
    }

    /// `task_running_tick()`
    fn tick(&self, task: &Arc<Task>, cur_tick: u64) {
        if !task.is_active() {
            // if the task has been expired
            task.set_need_resched();
            return;
        }

        if task.is_real_time() {
            self.tick_rt(task, cur_tick);
        } else {
            self.tick_normal(task, cur_tick);
        }
    }
}
