// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;
use core::time::Duration;

use aster_frame::sync::{guard_lock, MutexGuard, SpinLock};

use super::Pauser;
use crate::Errno;

/// The lock error type for situations where the condvar is not notified.
pub enum LockErr<Guard> {
    Timeout(Guard),
    Interrupted(Guard),
    Unknown(Guard),
}
/// LockResult, different from Rust std.
/// The result of a lock operation.
pub type LockResult<Guard> = Result<Guard, LockErr<Guard>>;

impl<Guard> LockErr<Guard> {
    pub fn into_guard(self) -> Guard {
        match self {
            LockErr::Timeout(guard) => guard,
            LockErr::Interrupted(guard) => guard,
            LockErr::Unknown(guard) => guard,
        }
    }
}

type WaitCounter = u64;
type NotifyCounter = u64;

/// A condition variable.
/// pauser: The pauser object used to pause and resume threads.
/// counter: The counter (WaitCounter, NotifyCounter).
pub struct Condvar {
    pauser: Arc<Pauser>,
    counter: SpinLock<(WaitCounter, NotifyCounter)>,
}

impl Condvar {
    /// Creates a new condition variable.
    pub fn new() -> Self {
        Condvar {
            pauser: Pauser::new(),
            counter: SpinLock::new((0, 0)),
        }
    }

    /// Waits for the condition variable to be signaled or broadcasted.
    pub fn wait<'a, T>(&self, guard: MutexGuard<'a, T>) -> LockResult<MutexGuard<'a, T>> {
        let lock = guard_lock(&guard);
        drop(guard);
        let cond = || {
            // Check if the notify counter is greater than 0.
            let mut counter = self.counter.lock_irq_disabled();
            if counter.1 > 0 {
                // Decrement the notify counter.
                counter.1 -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock_irq_disabled();
            counter.0 += 1;
        }
        let res = self.pauser.pause_until(cond);
        match res {
            // OK(): The thread is woken up normally
            Ok(_) => Ok(lock.lock()),
            Err(error) => {
                let mut counter = self.counter.lock_irq_disabled();
                counter.0 -= 1;
                match error.error() {
                    Errno::EINTR => Err(LockErr::Interrupted(lock.lock())),
                    _ => Err(LockErr::Unknown(lock.lock())),
                }
            }
        }
    }

    /// Waits for the condition variable to be signaled or broadcasted, or a timeout to elapse.
    /// bool is true if the timeout is reached.
    pub fn wait_timeout<'a, T>(
        &self,
        guard: MutexGuard<'a, T>,
        timeout: Duration,
    ) -> LockResult<(MutexGuard<'a, T>, bool)> {
        let lock = guard_lock(&guard);
        drop(guard);
        let cond = || {
            // Check if the notify counter is greater than 0.
            let mut counter = self.counter.lock_irq_disabled();
            if counter.1 > 0 {
                // Decrement the notify counter.
                counter.1 -= 1;
                Some(())
            } else {
                None
            }
        };
        {
            let mut counter = self.counter.lock_irq_disabled();
            counter.0 += 1;
        }
        // Wait until the condition becomes true, we're explicitly woken up, or the timeout elapses.
        let res = self.pauser.pause_until_or_timeout(cond, &timeout);
        match res {
            // OK(): The thread is woken up normally
            Ok(_) => Ok((lock.lock(), false)),
            Err(error) => {
                let mut counter = self.counter.lock_irq_disabled();
                counter.0 -= 1;
                match error.error() {
                    Errno::EINTR => Err(LockErr::Interrupted((lock.lock(), false))),
                    Errno::ETIME => Err(LockErr::Timeout((lock.lock(), true))),
                    _ => Err(LockErr::Unknown((lock.lock(), false))),
                }
            }
        }
    }

    /// Wait for the condition to become true, or until the timeout elapses, or until the condition is explicitly woken up.
    /// bool is true if the timeout is reached.
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
                Err(LockErr::Interrupted(guard)) => return Err(LockErr::Interrupted(guard)),
                Err(LockErr::Unknown(guard)) => return Err(LockErr::Unknown(guard)),
            }
        }
    }

    /// Wait for the condition to become true, and until the condition is explicitly woken up or interupted.
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
                Err(LockErr::Interrupted(guard)) => return Err(LockErr::Interrupted(guard)),
                Err(LockErr::Unknown(guard)) => return Err(LockErr::Unknown(guard)),
                _ => unreachable!(),
            }
        }
    }

    /// Wakes up one task waiting on the condition variable.
    pub fn notify_one(&self) {
        let mut counter = self.counter.lock_irq_disabled();
        if counter.0 == 0 {
            return;
        }
        counter.1 += 1;
        self.pauser.resume_one();
        counter.0 -= 1;
    }

    /// Wakes up all tasks waiting on the condition variable.
    pub fn notify_all(&self) {
        let mut counter = self.counter.lock_irq_disabled();
        if counter.0 == 0 {
            return;
        }
        counter.1 = counter.0;
        self.pauser.resume_all();
        counter.0 = 0;
    }
}
