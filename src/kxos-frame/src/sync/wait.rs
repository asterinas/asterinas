/// A wait queue.
///
/// One may wait on a wait queue to put its executing thread to sleep.
/// Multiple threads may be the waiters of a wait queue.
/// Other threads may invoke the `wake`-family methods of a wait queue to
/// wake up one or many waiter threads.
pub struct WaitQueue {}

impl WaitQueue {
    /// Creates a new instance.
    pub fn new() -> Self {
        todo!()
    }

    /// Wait until some condition becomes true.
    ///
    /// This method takes a closure that tests a user-given condition.
    /// The method only returns if the condition becomes true.
    /// A waker thread should first make the condition true, then invoke the
    /// `wake`-family method. This ordering is important to ensure that waiter
    /// threads do not lose any wakeup notifiations.
    ///
    /// By taking a condition closure, this wait-wakeup mechanism becomes
    /// more efficient and robust.
    pub fn wait_until<F>(&self, mut cond: F)
    where
        F: FnMut() -> bool,
    {
        let waiter = Waiter::new();
        self.enqueue(&waiter);
        loop {
            if (cond)() {
                self.dequeue(&waiter);
                break;
            }
            waiter.wait();
        }
        self.dequeue(&waiter);
    }

    /// Wake one waiter thread, if there is one.
    pub fn wake_one(&self) {
        todo!()
    }

    /// Wake all waiter threads.
    pub fn wake_all(&self) {
        todo!()
    }

    fn enqueue(&self, waiter: &Waiter) {
        todo!()
    }
    fn dequeue(&self, waiter: &Waiter) {
        todo!()
    }
}

struct Waiter {}

impl Waiter {
    pub fn new() -> Self {
        todo!()
    }

    pub fn wait(&self) {
        todo!()
    }
}
