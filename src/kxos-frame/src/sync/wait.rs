use alloc::collections::VecDeque;
use spin::mutex::Mutex;

use crate::{debug, task::Task};

/// A wait queue.
///
/// One may wait on a wait queue to put its executing thread to sleep.
/// Multiple threads may be the waiters of a wait queue.
/// Other threads may invoke the `wake`-family methods of a wait queue to
/// wake up one or many waiter threads.
pub struct WaitQueue<D: Clone + Eq + PartialEq> {
    waiters: Mutex<VecDeque<Waiter<D>>>,
}

impl<D: Clone + Eq + PartialEq> WaitQueue<D> {
    /// Creates a new instance.
    pub fn new() -> Self {
        WaitQueue {
            waiters: Mutex::new(VecDeque::new()),
        }
    }

    /// Wait until some condition becomes true.
    ///
    /// This method takes a closure that tests a user-given condition.
    /// The method only returns if the condition returns Some(_).
    /// A waker thread should first make the condition true, then invoke the
    /// `wake`-family method. This ordering is important to ensure that waiter
    /// threads do not lose any wakeup notifiations.
    ///
    /// By taking a condition closure, this wait-wakeup mechanism becomes
    /// more efficient and robust.
    pub fn wait_until<F, R>(&self, data: D, mut cond: F) -> R
    where
        F: FnMut() -> Option<R>,
    {
        let waiter = Waiter::new(data);
        self.enqueue(&waiter);
        loop {
            if let Some(r) = cond() {
                self.dequeue(&waiter);
                return r;
            }
            waiter.wait();
        }
    }

    /// Wake one waiter thread, if there is one.
    pub fn wake_one(&self) {
        if let Some(waiter) = self.waiters.lock().front_mut() {
            waiter.wake_up();
        }
    }

    /// Wake all waiter threads.
    pub fn wake_all(&self) {
        self.waiters.lock().iter_mut().for_each(|waiter| {
            waiter.wake_up();
        });
    }

    /// Wake all waiters if given condition returns true.
    /// The condition will check the data carried by waiter if it satisfy some relation with cond_data
    pub fn wake_all_on_condition<F, C>(&self, cond_data: &C, cond: F)
    where
        F: Fn(&D, &C) -> bool,
    {
        self.waiters.lock().iter_mut().for_each(|waiter| {
            if cond(waiter.data(), cond_data) {
                waiter.wake_up()
            }
        })
    }

    fn enqueue(&self, waiter: &Waiter<D>) {
        self.waiters.lock().push_back(waiter.clone());
    }
    fn dequeue(&self, waiter: &Waiter<D>) {
        let mut waiters_lock = self.waiters.lock();
        let len = waiters_lock.len();
        let mut index = 0;
        for i in 0..len {
            if waiters_lock[i] == *waiter {
                index = i;
                break;
            }
        }
        waiters_lock.remove(index);
        drop(waiters_lock);
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Waiter<D: Clone + Eq + PartialEq> {
    is_woken_up: bool,
    data: D,
}

impl<D: Clone + Eq + PartialEq> Waiter<D> {
    pub fn new(data: D) -> Self {
        Waiter {
            is_woken_up: false,
            data,
        }
    }

    pub fn wait(&self) {
        while !self.is_woken_up {
            // yield the execution, to allow other task to contine
            debug!("Waiter: wait");
            Task::yield_now();
        }
    }

    pub fn wake_up(&mut self) {
        self.is_woken_up = true;
    }

    pub fn data(&self) -> &D {
        &self.data
    }
}
