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

#[cfg(ktest)]
mod test {
    use core::sync::atomic::AtomicUsize;

    use ostd::{prelude::ktest, task::Task};

    use super::*;
    use crate::{
        process::{
            Credentials, Uid,
            posix_thread::{AsPosixThread, SleepingState},
            signal::{
                constants::{SIGKILL, SIGUSR1},
                signals::kernel::KernelSignal,
            },
            test::{new_process, new_task},
        },
        thread::{AsThread, Thread},
    };

    fn wait_until_sleeping(task: &Task) {
        for _ in 0..10000 {
            if matches!(
                task.as_posix_thread().unwrap().sleeping_state(),
                SleepingState::Interruptible | SleepingState::Uninterruptible
            ) {
                return;
            }
            assert!(!task.as_thread().unwrap().is_exited());
            Thread::yield_now();
        }
        panic!("credential waiter did not sleep");
    }

    #[ktest]
    fn exec_wait_requests_unconditional_restart() {
        let (process, vmar) = new_process();
        let guard = process.cred_guard().lock_for_exec().unwrap();
        let result = Arc::new(AtomicUsize::new(0));
        let task = new_task(&process, &vmar, {
            let process = process.clone();
            let result = result.clone();
            move || {
                if let Err(err) = process.cred_guard().lock_for_exec() {
                    result.store(err.error() as usize, Ordering::Release);
                }
            }
        });
        task.as_thread().unwrap().run();
        wait_until_sleeping(&task);
        task.as_posix_thread()
            .unwrap()
            .enqueue_signal(Box::new(KernelSignal::new(SIGUSR1)));
        task.as_thread().unwrap().join();
        // The waiter must return while the holder still owns the guard.
        assert_eq!(
            result.load(Ordering::Acquire),
            Errno::ERESTARTNOINTR as usize
        );
        assert!(process.cred_guard().try_lock().is_none());
        drop(guard);
        assert!(process.cred_guard().try_lock().is_some());
    }

    #[ktest]
    fn seccomp_wait_ignores_ordinary_signal_but_sigkill_cancels() {
        let (process, vmar) = new_process();
        let guard = process.cred_guard().lock_for_exec().unwrap();
        let result = Arc::new(AtomicUsize::new(0));
        let task = new_task(&process, &vmar, {
            let process = process.clone();
            let result = result.clone();
            move || {
                if let Err(err) = process.cred_guard().lock_for_seccomp() {
                    result.store(err.error() as usize, Ordering::Release);
                }
            }
        });
        // Pending before entry: an interruptible wait would return immediately.
        task.as_posix_thread()
            .unwrap()
            .enqueue_signal(Box::new(KernelSignal::new(SIGUSR1)));
        task.as_thread().unwrap().run();
        wait_until_sleeping(&task);
        assert_eq!(result.load(Ordering::Acquire), 0);
        task.as_posix_thread()
            .unwrap()
            .enqueue_signal(Box::new(KernelSignal::new(SIGKILL)));
        task.as_thread().unwrap().join();
        assert_eq!(result.load(Ordering::Acquire), Errno::EINTR as usize);
        assert!(process.cred_guard().try_lock().is_none());
        drop(guard);
        assert!(process.cred_guard().try_lock().is_some());
    }

    #[ktest]
    fn unlock_wakes_exec_and_seccomp_waiters() {
        let (process, vmar) = new_process();
        let guard = process.cred_guard().lock_for_exec().unwrap();
        let finished = Arc::new(AtomicUsize::new(0));
        let tasks: Vec<_> = [PauseReason::Sleep, PauseReason::Killable]
            .into_iter()
            .map(|reason| {
                let process_clone = process.clone();
                let finished = finished.clone();
                new_task(&process, &vmar, move || {
                    let _guard = match reason {
                        PauseReason::Sleep => process_clone.cred_guard().lock_for_exec(),
                        _ => process_clone.cred_guard().lock_for_seccomp(),
                    }
                    .unwrap();
                    finished.fetch_add(1, Ordering::Release);
                })
            })
            .collect();
        for task in &tasks {
            task.as_thread().unwrap().run();
            wait_until_sleeping(task);
        }
        assert_eq!(finished.load(Ordering::Acquire), 0);
        drop(guard);
        for task in &tasks {
            task.as_thread().unwrap().join();
        }
        assert_eq!(finished.load(Ordering::Acquire), 2);
    }

    #[ktest]
    fn sync_cannot_change_privileges_between_exec_prepare_and_commit() {
        let (process, vmar) = new_process();
        let credentials: Credentials = Credentials::new_root();
        credentials.set_uid(Uid::new(1000)).unwrap();
        let guard = process.cred_guard().lock_for_exec().unwrap();
        let prepared = credentials
            .prepare_exec_cred(None, Some(Uid::new_root()), None)
            .unwrap();
        let finished = Arc::new(AtomicBool::new(false));
        let task = new_task(&process, &vmar, {
            let process = process.clone();
            let credentials = credentials.dup();
            let finished = finished.clone();
            move || {
                let _guard = process.cred_guard().lock_for_seccomp().unwrap();
                credentials.set_no_new_privs();
                finished.store(true, Ordering::Release);
            }
        });
        task.as_thread().unwrap().run();
        wait_until_sleeping(&task);
        assert!(!credentials.no_new_privs());
        credentials.apply_exec_cred(prepared).unwrap();
        assert_eq!(credentials.euid(), Uid::new_root());
        assert!(!finished.load(Ordering::Acquire));
        drop(guard);
        task.as_thread().unwrap().join();
        assert!(finished.load(Ordering::Acquire));
        assert!(credentials.no_new_privs());

        // Reverse ordering: a completed TSYNC restriction suppresses setuid.
        credentials.set_uid(Uid::new(1000)).unwrap();
        let _guard = process.cred_guard().lock_for_exec().unwrap();
        let prepared = credentials
            .prepare_exec_cred(None, Some(Uid::new_root()), None)
            .unwrap();
        credentials.apply_exec_cred(prepared).unwrap();
        assert_eq!(credentials.euid(), Uid::new(1000));
    }
}
