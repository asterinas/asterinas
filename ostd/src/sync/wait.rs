// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::VecDeque, sync::Arc};
use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use super::{LocalIrqDisabled, SpinLock};
use crate::task::{scheduler, Task};

// # Explanation on the memory orders
//
// ```
// [CPU 1 (the waker)]     [CPU 2 (the waiter)]
// cond = true;
// wake_up();
//                         wait();
//                         if cond { /* .. */ }
// ```
//
// As soon as the waiter is woken up by the waker, it must see the true condition. This is
// trivially satisfied if `wake_up()` and `wait()` synchronize with a lock. But if they synchronize
// with an atomic variable, `wake_up()` must access the variable with `Ordering::Release` and
// `wait()` must access the variable with `Ordering::Acquire`.
//
// Examples of `wake_up()`:
//  - `WaitQueue::wake_one()`
//  - `WaitQueue::wake_all()`
//  - `Waker::wake_up()`
//
// Examples of `wait()`:
//  - `WaitQueue::wait_until()`
//  - `Waiter::wait()`
//  - `Waiter::drop()`
//
// Note that dropping a waiter must be treated as a `wait()` with zero timeout, because we need to
// make sure that the wake event isn't lost in this case.

/// A wait queue.
///
/// One may wait on a wait queue to put its executing thread to sleep.
/// Multiple threads may be the waiters of a wait queue.
/// Other threads may invoke the `wake`-family methods of a wait queue to
/// wake up one or many waiting threads.
pub struct WaitQueue {
    // A copy of `wakers.len()`, used for the lock-free fast path in `wake_one` and `wake_all`.
    num_wakers: AtomicU32,
    wakers: SpinLock<VecDeque<Arc<Waker>>, LocalIrqDisabled>,
}

impl WaitQueue {
    /// Creates a new, empty wait queue.
    pub const fn new() -> Self {
        WaitQueue {
            num_wakers: AtomicU32::new(0),
            wakers: SpinLock::new(VecDeque::new()),
        }
    }

    /// Waits until some condition is met.
    ///
    /// This method takes a closure that tests a user-given condition.
    /// The method only returns if the condition returns `Some(_)`.
    /// A waker thread should first make the condition `Some(_)`, then invoke the
    /// `wake`-family method. This ordering is important to ensure that waiter
    /// threads do not lose any wakeup notifications.
    ///
    /// By taking a condition closure, this wait-wakeup mechanism becomes
    /// more efficient and robust.
    pub fn wait_until<F, R>(&self, mut cond: F) -> R
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return res;
        }

        let (waiter, _) = Waiter::new_pair();
        let cond = || {
            self.enqueue(waiter.waker());
            cond()
        };
        waiter
            .wait_until_or_cancelled(cond, || Ok::<(), ()>(()))
            .unwrap()
    }

    /// Wakes up one waiting thread, if there is one at the point of time when this method is
    /// called, returning whether such a thread was woken up.
    pub fn wake_one(&self) -> bool {
        // Fast path
        if self.is_empty() {
            return false;
        }

        loop {
            let mut wakers = self.wakers.lock();
            let Some(waker) = wakers.pop_front() else {
                return false;
            };
            self.num_wakers.fetch_sub(1, Ordering::Release);
            // Avoid holding lock when calling `wake_up`
            drop(wakers);

            if waker.wake_up() {
                return true;
            }
        }
    }

    /// Wakes up all waiting threads, returning the number of threads that were woken up.
    pub fn wake_all(&self) -> usize {
        // Fast path
        if self.is_empty() {
            return 0;
        }

        let mut num_woken = 0;

        loop {
            let mut wakers = self.wakers.lock();
            let Some(waker) = wakers.pop_front() else {
                break;
            };
            self.num_wakers.fetch_sub(1, Ordering::Release);
            // Avoid holding lock when calling `wake_up`
            drop(wakers);

            if waker.wake_up() {
                num_woken += 1;
            }
        }

        num_woken
    }

    fn is_empty(&self) -> bool {
        // On x86-64, this generates `mfence; mov`, which is exactly the right way to implement
        // atomic loading with `Ordering::Release`. It performs much better than naively
        // translating `fetch_add(0)` to `lock; xadd`.
        self.num_wakers.fetch_add(0, Ordering::Release) == 0
    }

    /// Enqueues the input [`Waker`] to the wait queue.
    #[doc(hidden)]
    pub fn enqueue(&self, waker: Arc<Waker>) {
        let mut wakers = self.wakers.lock();
        wakers.push_back(waker);
        self.num_wakers.fetch_add(1, Ordering::Acquire);
    }
}

impl Default for WaitQueue {
    fn default() -> Self {
        Self::new()
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
/// A waker can be created by calling [`Waiter::new_pair`]. This method creates an `Arc<Waker>` that can
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
            task: Task::current().unwrap().cloned(),
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

    /// Waits until some condition is met or the cancel condition becomes true.
    ///
    /// This method will return `Ok(_)` if the condition returns `Some(_)`, and will stop waiting
    /// if the cancel condition returns `Err(_)`. In this situation, this method will return the `Err(_)`
    /// generated by the cancel condition.
    pub fn wait_until_or_cancelled<F, R, FCancel, E>(
        &self,
        mut cond: F,
        cancel_cond: FCancel,
    ) -> core::result::Result<R, E>
    where
        F: FnMut() -> Option<R>,
        FCancel: Fn() -> core::result::Result<(), E>,
    {
        loop {
            if let Some(res) = cond() {
                return Ok(res);
            };

            if let Err(e) = cancel_cond() {
                // Close the waker and check again to avoid missing a wake event.
                self.waker.close();
                return cond().ok_or(e);
            }

            self.wait();
        }
    }

    /// Gets the associated [`Waker`] of the current waiter.
    pub fn waker(&self) -> Arc<Waker> {
        self.waker.clone()
    }

    /// Returns the task that the associated waker will attempt to wake up.
    pub fn task(&self) -> &Arc<Task> {
        &self.waker.task
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
        if self.has_woken.swap(true, Ordering::Release) {
            return false;
        }
        scheduler::unpark_target(self.task.clone());

        true
    }

    fn do_wait(&self) {
        while !self.has_woken.swap(false, Ordering::Acquire) {
            scheduler::park_current(|| self.has_woken.load(Ordering::Acquire));
        }
    }

    fn close(&self) {
        // This must use `Ordering::Acquire`, although we do not care about the return value. See
        // the memory order explanation at the top of the file for details.
        let _ = self.has_woken.swap(true, Ordering::Acquire);
    }
}

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::{prelude::*, task::TaskOptions};

    fn queue_wake<F>(wake: F)
    where
        F: Fn(&WaitQueue) + Sync + Send + 'static,
    {
        let queue = Arc::new(WaitQueue::new());
        let queue_cloned = queue.clone();

        let cond = Arc::new(AtomicBool::new(false));
        let cond_cloned = cond.clone();

        TaskOptions::new(move || {
            Task::yield_now();

            cond_cloned.store(true, Ordering::Relaxed);
            wake(&queue_cloned);
        })
        .data(())
        .spawn()
        .unwrap();

        queue.wait_until(|| cond.load(Ordering::Relaxed).then_some(()));

        assert!(cond.load(Ordering::Relaxed));
    }

    #[ktest]
    fn queue_wake_one() {
        queue_wake(|queue| {
            queue.wake_one();
        });
    }

    #[ktest]
    fn queue_wake_all() {
        queue_wake(|queue| {
            queue.wake_all();
        });
    }

    #[ktest]
    fn waiter_wake_twice() {
        let (_waiter, waker) = Waiter::new_pair();

        assert!(waker.wake_up());
        assert!(!waker.wake_up());
    }

    #[ktest]
    fn waiter_wake_drop() {
        let (waiter, waker) = Waiter::new_pair();

        drop(waiter);
        assert!(!waker.wake_up());
    }

    #[ktest]
    fn waiter_wake_async() {
        let (waiter, waker) = Waiter::new_pair();

        let cond = Arc::new(AtomicBool::new(false));
        let cond_cloned = cond.clone();

        TaskOptions::new(move || {
            Task::yield_now();

            cond_cloned.store(true, Ordering::Relaxed);
            assert!(waker.wake_up());
        })
        .data(())
        .spawn()
        .unwrap();

        waiter.wait();

        assert!(cond.load(Ordering::Relaxed));
    }

    #[ktest]
    fn waiter_wake_reorder() {
        let (waiter, waker) = Waiter::new_pair();

        let cond = Arc::new(AtomicBool::new(false));
        let cond_cloned = cond.clone();

        let (waiter2, waker2) = Waiter::new_pair();

        let cond2 = Arc::new(AtomicBool::new(false));
        let cond2_cloned = cond2.clone();

        TaskOptions::new(move || {
            Task::yield_now();

            cond2_cloned.store(true, Ordering::Relaxed);
            assert!(waker2.wake_up());

            Task::yield_now();

            cond_cloned.store(true, Ordering::Relaxed);
            assert!(waker.wake_up());
        })
        .data(())
        .spawn()
        .unwrap();

        waiter.wait();
        assert!(cond.load(Ordering::Relaxed));

        waiter2.wait();
        assert!(cond2.load(Ordering::Relaxed));
    }
}
