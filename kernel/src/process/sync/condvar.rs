// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use alloc::sync::Arc;
use core::time::Duration;

use ostd::sync::{MutexGuard, SpinLock, WaitQueue};

use crate::time::wait::WaitTimeout;

/// Represents potential errors during lock operations on synchronization primitives,
/// specifically for operations associated with a `Condvar` (Condition Variable).
pub enum LockErr<Guard> {
    Timeout(Guard),
    Unknown(Guard),
}

/// LockResult, different from Rust std.
/// The result of a lock operation.
pub type LockResult<Guard> = Result<Guard, LockErr<Guard>>;

impl<Guard> LockErr<Guard> {
    pub fn into_guard(self) -> Guard {
        match self {
            LockErr::Timeout(guard) => guard,
            LockErr::Unknown(guard) => guard,
        }
    }
}

/// A `Condvar` (Condition Variable) is a synchronization primitive that can block threads
/// until a certain condition becomes true.
///
/// Although a `Condvar` can block threads, it is primarily used to achieve thread synchronization.
/// Threads waiting on a `Condvar` must acquire a mutex before proceeding. This setup is commonly
/// used with a shared mutable state to ensure safe concurrent access. A typical use involves one
/// or more threads waiting for a condition to become true to proceed with their operations.
///
/// # Usage
///
/// Pair a `Condvar` with a `Mutex` to allow threads to wait for certain conditions safely.
/// A waiting thread will sleep and atomically release the associated mutex.
/// Another thread can then update the shared state and notify the `Condvar`, allowing the
/// waiting thread to reacquire the mutex and proceed.
///
/// ## Example
///
/// This example demonstrates how a `Condvar` can synchronize threads:
///
/// ```rust
/// use alloc::sync::Arc;
/// use ostd::sync::Mutex;
/// use crate::{process::sync::Condvar, thread::kernel_thread::Thread};
///
/// // Initializing a shared condition between threads
/// let pair = Arc::new((Mutex::new(false), Condvar::new()));
/// let pair2 = Arc::clone(&pair);
///
/// // Spawning a new kernel thread to change a shared state and notify the Condvar
/// ThreadOptions::new(move || {
///     let (lock, cvar) = &*pair2;
///     Thread::yield_now();
///     let mut started = lock.lock();
///     *started = true; // Modifying the shared state
///     cvar.notify_one(); // Notifying one waiting thread
/// })
/// .spawn();
///
/// // Main thread waiting for the shared state to be set to true
/// {
///     let (lock, cvar) = &*pair;
///     let mut started = lock.lock();
///     while !*started {
///         started = cvar.wait(started).unwrap_or_else(|err| err.into_guard());
///     }
/// }
/// ```
///
/// In this example, the main thread and a child thread synchronize access to a boolean flag
/// using a `Mutex` and a `Condvar`.
/// The main thread waits for the flag to be set to `true`,
/// utilizing the `Condvar` to sleep efficiently until the condition is met.
pub struct Condvar {
    waitqueue: Arc<WaitQueue>,
    counter: SpinLock<Inner>,
}

struct Inner {
    waiter_count: u64,
    notify_count: u64,
}

impl Condvar {
    /// Creates a new condition variable.
    pub fn new() -> Self {
        Condvar {
            waitqueue: Arc::new(WaitQueue::new()),
            counter: SpinLock::new(Inner {
                waiter_count: 0,
                notify_count: 0,
            }),
        }
    }

    /// Atomically releases the given `MutexGuard`,
    /// blocking the current thread until the condition variable
    /// is notified, after which the mutex will be reacquired.
    ///
    /// Returns a new `MutexGuard` if the operation is successful,
    /// or returns the provided guard
    /// within a `LockErr` if the waiting operation fails.
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        let cond = || {
            // Check if the notify counter is greater than 0.
            let mut counter = self.counter.lock();
            if counter.notify_count > 0 {
                // Decrement the notify counter.
                counter.notify_count -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock();
            counter.waiter_count += 1;
        }
        let lock = MutexGuard::get_lock(&guard);
        drop(guard);
        self.waitqueue.wait_until(cond);
        Ok(lock.lock())
    }

    /// Waits for the condition variable to be signaled or broadcasted,
    /// or a timeout to elapse.
    /// bool is true if the timeout is reached.
    ///
    /// The function returns a tuple containing a `MutexGuard`
    /// and a boolean that is true if the timeout elapsed
    /// before the condition variable was notified.
    pub fn wait_timeout<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
    ) -> LockResult<(MutexGuard<'a, T>, bool)> {
        let cond = || {
            // Check if the notify counter is greater than 0.
            let mut counter = self.counter.lock();
            if counter.notify_count > 0 {
                // Decrement the notify counter.
                counter.notify_count -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock();
            counter.waiter_count += 1;
        }
        let lock = MutexGuard::get_lock(&guard);
        drop(guard);
        // Wait until the condition becomes true, we're explicitly woken up, or the timeout elapses.
        let res = self.waitqueue.wait_until_or_timeout(cond, &timeout);
        match res {
            Ok(()) => Ok((lock.lock(), false)),
            Err(_) => {
                let mut counter = self.counter.lock();
                counter.waiter_count -= 1;
                Err(LockErr::Timeout((lock.lock(), true)))
            }
        }
    }

    /// Wait for the condition to become true,
    /// or until the timeout elapses,
    /// or until the condition is explicitly woken up.
    /// bool is true if the timeout is reached.
    ///
    /// Similar to `wait_timeout`,
    /// it returns a tuple containing the `MutexGuard`
    /// and a boolean value indicating
    /// whether the wait operation terminated due to a timeout.
    pub fn wait_timeout_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        timeout: Duration,
        mut condition: F,
    ) -> LockResult<(MutexGuard<'a, T>, bool)>
    where
        F: FnMut(&mut T) -> bool,
    {
        loop {
            if !condition(&mut *guard) {
                return Ok((guard, false));
            }
            guard = match self.wait_timeout(guard, timeout) {
                Ok((guard, timeout_flag)) => guard,
                Err(LockErr::Timeout((guard, timeout_flag))) => {
                    return Err(LockErr::Timeout((guard, timeout_flag)))
                }
                Err(LockErr::Unknown(guard)) => return Err(LockErr::Unknown(guard)),
            }
        }
    }

    /// Wait for the condition to become true,
    /// and until the condition is explicitly woken up or interrupted.
    ///
    /// This function blocks until either the condition becomes false
    /// or the condition variable is explicitly notified.
    /// Returns the `MutexGuard` if the operation completes successfully.
    pub fn wait_while<'a, T, F>(
        &self,
        mut guard: MutexGuard<'a, T>,
        mut condition: F,
    ) -> LockResult<MutexGuard<'a, T>>
    where
        F: FnMut(&mut T) -> bool,
    {
        loop {
            if !condition(&mut *guard) {
                return Ok(guard);
            }
            guard = match self.wait(guard) {
                Ok(guard) => guard,
                Err(LockErr::Unknown(guard)) => return Err(LockErr::Unknown(guard)),
                _ => unreachable!(),
            }
        }
    }

    /// Wakes up one blocked thread waiting on this condition variable.
    ///
    /// If there is a waiting thread, it will be unblocked
    /// and allowed to reacquire the associated mutex.
    /// If no threads are waiting, this function is a no-op.
    pub fn notify_one(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count += 1;
        self.waitqueue.wake_one();
        counter.waiter_count -= 1;
    }

    /// Wakes up all blocked threads waiting on this condition variable.
    ///
    /// This method will unblock all waiting threads
    /// and they will be allowed to reacquire the associated mutex.
    /// If no threads are waiting, this function is a no-op.
    pub fn notify_all(&self) {
        let mut counter = self.counter.lock();
        if counter.waiter_count == 0 {
            return;
        }
        counter.notify_count = counter.waiter_count;
        self.waitqueue.wake_all();
        counter.waiter_count = 0;
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{prelude::*, sync::Mutex};

    use super::*;
    use crate::thread::{kernel_thread::ThreadOptions, Thread};

    #[ktest]
    fn test_condvar_wait() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = true;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let mut started = lock.lock();
            while !*started {
                started = cvar.wait(started).unwrap_or_else(|err| err.into_guard());
            }
            assert!(*started);
        }
    }

    #[ktest]
    fn test_condvar_wait_timeout() {
        let pair = Arc::new((Mutex::new(false), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = true;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let mut started = lock.lock();
            while !*started {
                (started, _) = cvar
                    .wait_timeout(started, Duration::from_secs(1))
                    .unwrap_or_else(|err| err.into_guard());
            }
            assert!(*started);
        }
    }

    #[ktest]
    fn test_condvar_wait_while() {
        let pair = Arc::new((Mutex::new(true), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = false;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let started = cvar
                .wait_while(lock.lock(), |started| *started)
                .unwrap_or_else(|err| err.into_guard());
            assert!(!*started);
        }
    }

    #[ktest]
    fn test_condvar_wait_timeout_while() {
        let pair = Arc::new((Mutex::new(true), Condvar::new()));
        let pair2 = Arc::clone(&pair);

        ThreadOptions::new(move || {
            Thread::yield_now();
            let (lock, cvar) = &*pair2;
            let mut started = lock.lock();
            *started = false;
            cvar.notify_one();
        })
        .spawn();

        {
            let (lock, cvar) = &*pair;
            let (started, _) = cvar
                .wait_timeout_while(lock.lock(), Duration::from_secs(1), |started| *started)
                .unwrap_or_else(|err| err.into_guard());
            assert!(!*started);
        }
    }
}
