// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, sync::Arc};
use core::time::Duration;

use super::SpinLock;
use crate::{
    arch::timer::{add_timeout_list, TIMER_FREQ},
    task::{add_task, current_task, schedule, Task, TaskStatus},
};

/// A wait queue.
///
/// One may wait on a wait queue to put its executing thread to sleep.
/// Multiple threads may be the waiters of a wait queue.
/// Other threads may invoke the `wake`-family methods of a wait queue to
/// wake up one or many waiter threads.
pub struct WaitQueue {
    waiters: SpinLock<VecDeque<Arc<Waiter>>>,
}

impl WaitQueue {
    pub const fn new() -> Self {
        WaitQueue {
            waiters: SpinLock::new(VecDeque::new()),
        }
    }

    /// Wait until some condition becomes true.
    ///
    /// This method takes a closure that tests a user-given condition.
    /// The method only returns if the condition returns Some(_).
    /// A waker thread should first make the condition Some(_), then invoke the
    /// `wake`-family method. This ordering is important to ensure that waiter
    /// threads do not lose any wakeup notifiations.
    ///
    /// By taking a condition closure, his wait-wakeup mechanism becomes
    /// more efficient and robust.
    pub fn wait_until<F, R>(&self, cond: F) -> R
    where
        F: FnMut() -> Option<R>,
    {
        self.do_wait(cond, None).unwrap()
    }

    /// Wait until some condition returns Some(_), or a given timeout is reached. If
    /// the condition does not becomes Some(_) before the timeout is reached, the
    /// function will return None.
    pub fn wait_until_or_timeout<F, R>(&self, cond: F, timeout: &Duration) -> Option<R>
    where
        F: FnMut() -> Option<R>,
    {
        self.do_wait(cond, Some(timeout))
    }

    fn do_wait<F, R>(&self, mut cond: F, timeout: Option<&Duration>) -> Option<R>
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return Some(res);
        }

        let waiter = Arc::new(Waiter::new());

        let timer_callback = timeout.map(|timeout| {
            let remaining_ticks = {
                // FIXME: We currently require 1000 to be a multiple of TIMER_FREQ, but
                // this may not hold true in the future, because TIMER_FREQ can be greater
                // than 1000. Then, the code need to be refactored.
                const_assert!(1000 % TIMER_FREQ == 0);

                let ms_per_tick = 1000 / TIMER_FREQ;

                // The ticks should be equal to or greater than timeout
                (timeout.as_millis() as u64 + ms_per_tick - 1) / ms_per_tick
            };

            add_timeout_list(remaining_ticks, waiter.clone(), |timer_call_back| {
                let waiter = timer_call_back
                    .data()
                    .downcast_ref::<Arc<Waiter>>()
                    .unwrap();
                waiter.wake_up();
            })
        });

        loop {
            if let Some(res) = cond() {
                if let Some(timer_callback) = timer_callback {
                    timer_callback.cancel();
                }

                return Some(res);
            };

            if let Some(ref timer_callback) = timer_callback
                && timer_callback.is_expired()
            {
                return cond();
            }

            self.enqueue(&waiter);
            waiter.wait();
        }
    }

    /// Wake up one waiting thread.
    pub fn wake_one(&self) {
        while let Some(waiter) = self.waiters.lock_irq_disabled().pop_front() {
            // Avoid holding lock when calling `wake_up`
            if waiter.wake_up() {
                return;
            }
        }
    }

    /// Wake up all waiting threads.
    pub fn wake_all(&self) {
        while let Some(waiter) = self.waiters.lock_irq_disabled().pop_front() {
            // Avoid holding lock when calling `wake_up`
            waiter.wake_up();
        }
    }

    pub fn is_empty(&self) -> bool {
        self.waiters.lock_irq_disabled().is_empty()
    }

    // Enqueue a waiter into current waitqueue. If waiter is exclusive, add to the back of waitqueue.
    // Otherwise, add to the front of waitqueue
    fn enqueue(&self, waiter: &Arc<Waiter>) {
        self.waiters.lock_irq_disabled().push_back(waiter.clone());
    }
}

struct Waiter {
    /// The `Task` held by the waiter.
    task: Arc<Task>,
}

impl Waiter {
    pub fn new() -> Self {
        Waiter {
            task: current_task().unwrap(),
        }
    }

    /// Wait until being woken up
    pub fn wait(&self) {
        debug_assert_eq!(
            self.task.inner_exclusive_access().task_status,
            TaskStatus::Runnable
        );
        self.task.inner_exclusive_access().task_status = TaskStatus::Sleeping;
        while self.task.inner_exclusive_access().task_status == TaskStatus::Sleeping {
            schedule();
        }
    }

    /// Wake up a waiting task.
    /// If the task is waiting before being woken, return true;
    /// Otherwise return false.
    pub fn wake_up(&self) -> bool {
        let mut task = self.task.inner_exclusive_access();
        if task.task_status == TaskStatus::Sleeping {
            task.task_status = TaskStatus::Runnable;

            // Avoid holding lock when doing `add_task`
            drop(task);

            add_task(self.task.clone());

            true
        } else {
            false
        }
    }
}
