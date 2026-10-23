// SPDX-License-Identifier: MPL-2.0

use core::{
    marker::PhantomData,
    sync::atomic::{AtomicBool, Ordering},
};

use ostd::sync::WaitQueue;

use crate::{prelude::*, process::signal::PauseReason};

/// Serializes exec credential changes with seccomp thread synchronization.
///
/// Lock order: credential guard -> process task set -> per-thread seccomp state.
/// The guard must be released before reporting the ptrace exec event.
/// This lock protects a transaction across existing independently locked objects;
/// it does not provide access to otherwise unsynchronized memory.
pub(crate) struct CredGuardMutex {
    locked: AtomicBool,
    waiters: WaitQueue,
}

impl CredGuardMutex {
    pub(crate) fn new() -> Self {
        Self {
            locked: AtomicBool::new(false),
            waiters: WaitQueue::new(),
        }
    }

    /// Acquires the guard for exec, restarting the syscall if a signal interrupts.
    pub(crate) fn lock_for_exec(&self) -> Result<CredGuard<'_>> {
        self.waiters.pause_until(|| self.try_lock()).map_err(|_| {
            Error::with_message(Errno::ERESTARTNOINTR, "exec credential wait interrupted")
        })
    }

    /// Acquires the guard for TSYNC, allowing only SIGKILL to interrupt the wait.
    pub(crate) fn lock_for_seccomp(&self) -> Result<CredGuard<'_>> {
        self.waiters
            .pause_until_by(|| self.try_lock(), PauseReason::Killable)
    }

    fn try_lock(&self) -> Option<CredGuard<'_>> {
        self.locked
            .compare_exchange(false, true, Ordering::Acquire, Ordering::Relaxed)
            .ok()
            .map(|_| CredGuard {
                mutex: self,
                _not_send: PhantomData,
            })
    }
}

/// Releases a process credential transaction when dropped.
pub(crate) struct CredGuard<'a> {
    mutex: &'a CredGuardMutex,
    _not_send: PhantomData<MutexGuard<'a, ()>>,
}

impl Drop for CredGuard<'_> {
    fn drop(&mut self) {
        self.mutex.locked.store(false, Ordering::Release);
        self.mutex.waiters.wake_all();
    }
}
