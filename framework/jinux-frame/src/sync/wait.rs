use core::sync::atomic::{AtomicBool, Ordering};

use super::SpinLock;
use alloc::{collections::VecDeque, sync::Arc};
use bitflags::bitflags;

use crate::task::{add_task, current_task, schedule, Task, TaskStatus};

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
    /// Creates a new instance.
    pub fn new() -> Self {
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
    pub fn wait_until<F, R>(&self, mut cond: F) -> R
    where
        F: FnMut() -> Option<R>,
    {
        if let Some(res) = cond() {
            return res;
        }
        let waiter = Arc::new(Waiter::new());
        self.enqueue(&waiter);
        loop {
            if let Some(res) = cond() {
                self.dequeue(&waiter);
                return res;
            };
            waiter.wait();
        }
    }

    /// Wake one waiter thread, if there is one.
    pub fn wake_one(&self) {
        if let Some(waiter) = self.waiters.lock_irq_disabled().front() {
            waiter.wake_up();
        }
    }

    /// Wake all not-exclusive waiter threads and at most one exclusive waiter.
    pub fn wake_all(&self) {
        for waiter in self.waiters.lock_irq_disabled().iter() {
            waiter.wake_up();
            if waiter.is_exclusive() {
                break;
            }
        }
    }

    // Enqueue a waiter into current waitqueue. If waiter is exclusive, add to the back of waitqueue.
    // Otherwise, add to the front of waitqueue
    fn enqueue(&self, waiter: &Arc<Waiter>) {
        if waiter.is_exclusive() {
            self.waiters.lock_irq_disabled().push_back(waiter.clone())
        } else {
            self.waiters.lock_irq_disabled().push_front(waiter.clone());
        }
    }

    fn dequeue(&self, waiter: &Arc<Waiter>) {
        self.waiters
            .lock_irq_disabled()
            .retain(|waiter_| !Arc::ptr_eq(waiter_, waiter))
    }
}

struct Waiter {
    /// Whether the
    is_woken_up: AtomicBool,
    /// To respect different wait condition
    flag: WaiterFlag,
    /// The `Task` held by the waiter.
    task: Arc<Task>,
}

impl Waiter {
    pub fn new() -> Self {
        Waiter {
            is_woken_up: AtomicBool::new(false),
            flag: WaiterFlag::empty(),
            task: current_task().unwrap(),
        }
    }

    /// make self into wait status until be called wake up
    pub fn wait(&self) {
        self.is_woken_up.store(false, Ordering::SeqCst);
        self.task.inner_exclusive_access().task_status = TaskStatus::Sleeping;
        while !self.is_woken_up.load(Ordering::SeqCst) {
            schedule();
        }
        self.task.inner_exclusive_access().task_status = TaskStatus::Runnable;
    }

    pub fn is_woken_up(&self) -> bool {
        self.is_woken_up.load(Ordering::SeqCst)
    }

    pub fn wake_up(&self) {
        if let Ok(false) =
            self.is_woken_up
                .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
        {
            add_task(self.task.clone());
        }
    }

    pub fn is_exclusive(&self) -> bool {
        self.flag.contains(WaiterFlag::EXCLUSIVE)
    }
}

bitflags! {
    pub struct WaiterFlag: u32 {
        const EXCLUSIVE         = 1 << 0;
        const INTERRUPTIABLE    = 1 << 1;
    }
}
