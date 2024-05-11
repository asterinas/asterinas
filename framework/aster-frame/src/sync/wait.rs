// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, sync::Arc};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::SpinLock;
use crate::task::{add_task, current_task, schedule, Task, TaskStatus};

/// A wait queue.
///
/// One may wait on a wait queue to put its executing thread to sleep.
/// Multiple threads may be the waiters of a wait queue.
/// Other threads may invoke the `wake`-family methods of a wait queue to
/// wake up one or many waiter threads.
pub struct WaitQueue {
    // A copy of `wakers.len()`, used for the lock-free fast path in `wake_one` and `wake_all`.
    num_wakers: AtomicU32,
    wakers: SpinLock<VecDeque<Arc<Waker>>>,
}

impl WaitQueue {
    pub const fn new() -> Self {
        WaitQueue {
            num_wakers: AtomicU32::new(0),
            wakers: SpinLock::new(VecDeque::new()),
        }
    }

    /// Wait until some condition becomes true.
    ///
    /// This method takes a closure that tests a user-given condition.
    /// The method only returns if the condition returns `Some(_)`.
    /// A waker thread should first make the condition `Some(_)`, then invoke the
    /// `wake`-family method. This ordering is important to ensure that waiter
    /// threads do not lose any wakeup notifications.
    ///
    /// By taking a condition closure, his wait-wakeup mechanism becomes
    /// more efficient and robust.
    pub fn wait_until<F, R>(&self, mut cond: F) -> R
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return res;
        }

        let (waiter, _) = Waiter::new_pair();

        self.wait_until_or_cancelled(cond, waiter, || false)
            .unwrap()
    }

    /// Wait until some condition becomes true or the cancel condition becomes true.
    ///
    /// This method will return `Some(_)` if the condition returns `Some(_)`, and will return
    /// the condition test result regardless what it is when the cancel condition becomes true.
    #[doc(hidden)]
    pub fn wait_until_or_cancelled<F, R, FCancel>(
        &self,
        mut cond: F,
        waiter: Waiter,
        cancel_cond: FCancel,
    ) -> Option<R>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> bool,
    {
        let waker = waiter.waker();

        loop {
            // Enqueue the waker before checking `cond()` to avoid races
            self.enqueue(waker.clone());

            if let Some(res) = cond() {
                return Some(res);
            };

            if cancel_cond() {
                // Drop the waiter and check again to avoid missing a wake event.
                drop(waiter);
                return cond();
            }

            waiter.wait();
        }
    }

    /// Wake up one waiting thread.
    pub fn wake_one(&self) {
        // Fast path
        if self.is_empty() {
            return;
        }

        loop {
            let mut wakers = self.wakers.lock_irq_disabled();
            let Some(waker) = wakers.pop_front() else {
                break;
            };
            self.num_wakers.fetch_sub(1, Ordering::Release);
            // Avoid holding lock when calling `wake_up`
            drop(wakers);

            if waker.wake_up() {
                return;
            }
        }
    }

    /// Wake up all waiting threads.
    pub fn wake_all(&self) {
        // Fast path
        if self.is_empty() {
            return;
        }

        loop {
            let mut wakers = self.wakers.lock_irq_disabled();
            let Some(waker) = wakers.pop_front() else {
                break;
            };
            self.num_wakers.fetch_sub(1, Ordering::Release);
            // Avoid holding lock when calling `wake_up`
            drop(wakers);

            waker.wake_up();
        }
    }

    /// Return whether the current wait queue is empty.
    pub fn is_empty(&self) -> bool {
        self.num_wakers.load(Ordering::Acquire) == 0
    }

    fn enqueue(&self, waker: Arc<Waker>) {
        let mut wakers = self.wakers.lock_irq_disabled();
        wakers.push_back(waker);
        self.num_wakers.fetch_add(1, Ordering::Release);
    }
}

/// A waiter that can put the current thread to sleep until it is woken up by the associated
/// [`Waker`].
///
/// By definition, a waiter belongs to the current thread, so it cannot be sent to another thread
/// and its reference cannot be shared between threads.
pub struct Waiter {
    waker: Arc<Waker>,
}

impl !Send for Waiter {}
impl !Sync for Waiter {}

/// A waker that can wake up the associated [`Waiter`].
///
/// A waker can be created by calling [`Waiter::new`]. This method creates an `Arc<Waker>` that can
/// be used across different threads.
pub struct Waker {
    has_woken: AtomicBool,
    task: Arc<Task>,
}

impl Waiter {
    /// Creates a waiter and its associated [`Waker`].
    pub fn new_pair() -> (Self, Arc<Waker>) {
        let waker = Arc::new(Waker {
            has_woken: AtomicBool::new(false),
            task: current_task().unwrap(),
        });
        let waiter = Self {
            waker: waker.clone(),
        };
        (waiter, waker)
    }

    /// Waits until the waiter is woken up by calling [`Waker::wake_up`] on the associated
    /// [`Waker`].
    ///
    /// This method returns immediately if the waiter has been woken since the end of the last call
    /// to this method (or since the waiter was created, if this method has not been called
    /// before). Otherwise, it puts the current thread to sleep until the waiter is woken up.
    pub fn wait(&self) {
        self.waker.do_wait();
    }

    /// Gets the associated [`Waker`] of the current waiter.
    pub fn waker(&self) -> Arc<Waker> {
        self.waker.clone()
    }
}

impl Drop for Waiter {
    fn drop(&mut self) {
        // When dropping the waiter, we need to close the waker to ensure that if someone wants to
        // wake up the waiter afterwards, they will perform a no-op.
        self.waker.close();
    }
}

impl Waker {
    /// Wakes up the associated [`Waiter`].
    ///
    /// This method returns `true` if the waiter is woken by this call. It returns `false` if the
    /// waiter has already been woken by a previous call to the method, or if the waiter has been
    /// dropped.
    ///
    /// Note that if this method returns `true`, it implies that the wake event will be properly
    /// delivered, _or_ that the waiter will be dropped after being woken. It's up to the caller to
    /// handle the latter case properly to avoid missing the wake event.
    pub fn wake_up(&self) -> bool {
        if self.has_woken.swap(true, Ordering::AcqRel) {
            return false;
        }

        let mut task = self.task.inner_exclusive_access();
        match task.task_status {
            TaskStatus::Sleepy => {
                task.task_status = TaskStatus::Runnable;
            }
            TaskStatus::Sleeping => {
                task.task_status = TaskStatus::Runnable;

                // Avoid holding the lock when doing `add_task`
                drop(task);
                add_task(self.task.clone());
            }
            _ => (),
        }

        true
    }

    fn do_wait(&self) {
        while !self.has_woken.load(Ordering::Acquire) {
            let mut task = self.task.inner_exclusive_access();
            // After holding the lock, check again to avoid races
            if self.has_woken.load(Ordering::Acquire) {
                break;
            }
            task.task_status = TaskStatus::Sleepy;
            drop(task);

            schedule();
        }

        self.has_woken.store(false, Ordering::Release);
    }

    fn close(&self) {
        self.has_woken.store(true, Ordering::Release);
    }
}
