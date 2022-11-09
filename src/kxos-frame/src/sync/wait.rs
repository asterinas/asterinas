use core::sync::atomic::{AtomicBool, Ordering};

use crate::prelude::*;
use alloc::{collections::VecDeque, sync::Arc};
use bitflags::bitflags;
use spin::mutex::Mutex;

use crate::task::schedule;

/// A wait queue.
///
/// One may wait on a wait queue to put its executing thread to sleep.
/// Multiple threads may be the waiters of a wait queue.
/// Other threads may invoke the `wake`-family methods of a wait queue to
/// wake up one or many waiter threads.
pub struct WaitQueue {
    waiters: Mutex<VecDeque<Arc<Waiter>>>,
}

impl WaitQueue {
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
    /// A waker thread should first make the condition Some(_), then invoke the
    /// `wake`-family method. This ordering is important to ensure that waiter
    /// threads do not lose any wakeup notifiations.
    ///
    /// By taking a condition closure, his wait-wakeup mechanism becomes
    /// more efficient and robust.
    pub fn wait_until<F, R>(&self, mut cond: F) -> Result<R>
    where
        F: FnMut() -> Result<Option<R>>,
    {
        let waiter = Arc::new(Waiter::new());
        self.enqueue(&waiter);
        loop {
            let ret_value = match cond() {
                Ok(Some(ret_value)) => Some(Ok(ret_value)),
                Ok(None) => None,
                Err(err) => Some(Err(err)),
            };
            if let Some(ret_value) = ret_value {
                waiter.set_finished();
                self.finish_wait();
                return ret_value;
            }
            waiter.wait();
        }
    }

    /// Wait on an waiter with data until the waiter is woken up.
    /// Note this func cannot be implemented with wait_until. This func always requires the waiter become woken.
    /// While wait_until does not check the waiter if cond is true.
    /// TODO: This function can take a timeout param further.
    // pub fn wait_on(&self, data: D) {
    //     let index = self
    //         .waiters
    //         .lock()
    //         .iter()
    //         .position(|waiter| *waiter.data() == data);
    //     if let Some(index) = index {
    //         let waiter = self.waiters.lock().iter().nth(index).unwrap().clone();
    //         waiter.wait();
    //     }
    // }

    /// Wake one waiter thread, if there is one.
    pub fn wake_one(&self) {
        if let Some(waiter) = self.waiters.lock().front() {
            waiter.wake_up();
        }
    }

    /// Wake all not-exclusive waiter threads and at most one exclusive waiter.
    pub fn wake_all(&self) {
        for waiter in self.waiters.lock().iter() {
            waiter.wake_up();
            if waiter.is_exclusive() {
                break;
            }
        }
    }

    /// Wake all waiters if given condition returns true.
    /// The condition will check the data carried by waiter if it satisfy some relation with cond_data
    // pub fn wake_all_on_condition<F, C>(&self, cond_data: &C, cond: F)
    // where
    //     F: Fn(&D, &C) -> bool,
    // {
    //     self.waiters.lock().iter().for_each(|waiter| {
    //         if cond(waiter.data(), cond_data) {
    //             waiter.wake_up()
    //         }
    //     })
    // }

    /// Wake at most max_count waiters if given condition is true.
    /// returns the number of woken waiters
    // pub fn batch_wake_and_deque<F, C>(&self, max_count: usize, cond_data: &C, cond: F) -> usize
    // where
    //     F: Fn(&D, &C) -> bool,
    // {
    //     let mut count = 0;
    //     let mut waiters_to_wake = Vec::new();
    //     self.waiters.lock().retain(|waiter| {
    //         if count >= max_count || waiter.is_woken_up() || !cond(waiter.data(), cond_data) {
    //             true
    //         } else {
    //             waiters_to_wake.push(waiter.clone());
    //             count += 1;
    //             false
    //         }
    //     });
    //     waiters_to_wake.into_iter().for_each(|waiter| {
    //         waiter.wake_up();
    //     });
    //     return count;
    // }

    /// create a waiter with given data, and enqueue
    // pub fn enqueue(&self) {
    //     let waiter = Arc::new(Waiter::new(data));
    //     self.enqueue_waiter(&waiter);
    // }

    /// dequeue a waiter with given data
    // pub fn dequeue(&self, data: D) {
    //     let waiter = Arc::new(Waiter::new(data));
    //     self.dequeue_waiter(&waiter);
    // }

    /// update the waiters data
    /// if cond(old_data, old_value) is true.
    /// The new data should be calculated by get_new_data(old_data, new_value).
    // pub fn update_waiters_data<F1, F2, C>(
    //     &self,
    //     cond: F1,
    //     old_value: &C,
    //     new_value: &C,
    //     get_new_data: F2,
    //     max_count: usize,
    // ) where
    //     F1: Fn(&C, &D) -> bool,
    //     F2: Fn(&D, &C) -> D,
    // {
    //     let mut waiters = self.waiters.lock();
    //     let len = waiters.len();
    //     let mut count = 0;
    //     for index in 0..len {
    //         let waiter = &waiters[index];
    //         let old_data = waiter.data();
    //         if cond(old_value, waiter.data()) {
    //             let new_data = get_new_data(old_data, new_value);
    //             let new_waiter = Arc::new(Waiter::new(new_data));
    //             waiters[index] = new_waiter;
    //             count += 1;
    //             if count >= max_count {
    //                 break;
    //             }
    //         }
    //     }
    // }

    /// remove waiters for which the cond returns true
    // pub fn remove_waiters<C, F>(&self, cond: F, cond_data: &C, max_count: usize) -> Vec<D>
    // where
    //     F: Fn(&D, &C) -> bool,
    // {
    //     let mut removed_waiters = Vec::new();
    //     let mut count = 0;
    //     self.waiters.lock().retain(|waiter| {
    //         let data = waiter.data();
    //         if count >= max_count || !cond(data, cond_data) {
    //             true
    //         } else {
    //             count += 1;
    //             removed_waiters.push(data.clone());
    //             false
    //         }
    //     });

    //     removed_waiters
    // }

    // enqueue a waiter into current waitqueue. If waiter is exclusive, add to the back of waitqueue.
    // Otherwise, add to the front of waitqueue
    fn enqueue(&self, waiter: &Arc<Waiter>) {
        if waiter.is_exclusive() {
            self.waiters.lock().push_back(waiter.clone())
        } else {
            self.waiters.lock().push_front(waiter.clone());
        }
    }

    /// removes all waiters that have finished wait
    fn finish_wait(&self) {
        self.waiters.lock().retain(|waiter| !waiter.is_finished())
    }

    // fn dequeue_waiter(&self, waiter_ref: &WaiterRef<D>) {
    //     let mut waiters_lock = self.waiters.lock();
    //     let index = waiters_lock
    //         .iter()
    //         .position(|waiter_| *waiter_ref.data() == *waiter_.data());
    //     if let Some(index) = index {
    //         waiters_lock.remove(index);
    //     }
    //     drop(waiters_lock);
    // }
}

#[derive(Debug)]
struct Waiter {
    /// Whether the
    is_woken_up: AtomicBool,
    /// To respect different wait condition
    flag: WaiterFlag,
    /// if the wait condition is ture, then the waiter is finished and can be removed from waitqueue
    wait_finished: AtomicBool,
}

impl Waiter {
    pub fn new() -> Self {
        Waiter {
            is_woken_up: AtomicBool::new(false),
            flag: WaiterFlag::empty(),
            wait_finished: AtomicBool::new(false),
        }
    }

    /// make self into wait status until be called wake up
    pub fn wait(&self) {
        self.is_woken_up.store(false, Ordering::SeqCst);
        while !self.is_woken_up.load(Ordering::SeqCst) {
            // yield the execution, to allow other task to continue
            schedule();
        }
    }

    pub fn is_woken_up(&self) -> bool {
        self.is_woken_up.load(Ordering::SeqCst)
    }

    pub fn wake_up(&self) {
        self.is_woken_up.store(true, Ordering::SeqCst);
    }

    pub fn set_finished(&self) {
        self.wait_finished.store(true, Ordering::SeqCst);
    }

    pub fn is_finished(&self) -> bool {
        self.wait_finished.load(Ordering::SeqCst)
    }

    pub fn is_exclusive(&self) -> bool {
        self.flag.contains(WaiterFlag::EXCLUSIVE)
    }
}

bitflags! {
    pub struct WaiterFlag: u32 {
        const EXCLUSIVE = 0x1;
        const INTERRUPTIABLE = 0x10;
    }
}
