// SPDX-License-Identifier: MPL-2.0

//! The process status.

use core::sync::atomic::{AtomicBool, AtomicU32, Ordering};

use ostd::sync::SpinLock;

use super::ExitCode;
use crate::process::{signal::sig_num::SigNum, WaitOptions};

/// The status of a process.
///
/// This maintains:
/// 1. Whether the process is a zombie (i.e., all its threads have exited);
/// 2. Whether the process is the vfork child, which shares the user-space virtual memory
///    with its parent process;
/// 3. The exit code of the process;
/// 4. Whether the process is stopped (by a signal or ptrace).
#[derive(Debug)]
pub struct ProcessStatus {
    is_zombie: AtomicBool,
    is_vfork_child: AtomicBool,
    exit_code: AtomicU32,
    stop_status: StopStatus,
}

impl Default for ProcessStatus {
    fn default() -> Self {
        Self {
            is_zombie: AtomicBool::new(false),
            is_vfork_child: AtomicBool::new(false),
            exit_code: AtomicU32::new(0),
            stop_status: StopStatus::new(),
        }
    }
}

impl ProcessStatus {
    /// Returns whether the process is a zombie process.
    pub fn is_zombie(&self) -> bool {
        // Use the `Acquire` memory order to make the exit code visible.
        self.is_zombie.load(Ordering::Acquire)
    }

    /// Sets the process to be a zombie process.
    ///
    /// This method should be called when the process completes its exit. The current thread must
    /// be the last thread in the process, so that no threads belonging to the process can run
    /// after it.
    pub(super) fn set_zombie(&self) {
        // Use the `Release` memory order to make the exit code visible.
        self.is_zombie.store(true, Ordering::Release);
    }
}

impl ProcessStatus {
    /// Returns whether the process is the vfork child.
    pub fn is_vfork_child(&self) -> bool {
        self.is_vfork_child.load(Ordering::Acquire)
    }

    /// Sets whether the process is the vfork child.
    pub fn set_vfork_child(&self, is_vfork_child: bool) {
        self.is_vfork_child.store(is_vfork_child, Ordering::Release);
    }
}

impl ProcessStatus {
    /// Returns the exit code.
    pub fn exit_code(&self) -> ExitCode {
        self.exit_code.load(Ordering::Relaxed)
    }

    /// Sets the exit code.
    pub(super) fn set_exit_code(&self, exit_code: ExitCode) {
        self.exit_code.store(exit_code, Ordering::Relaxed);
    }
}

impl ProcessStatus {
    pub(super) fn stop_status(&self) -> &StopStatus {
        &self.stop_status
    }
}

#[derive(Debug)]
pub(super) struct StopStatus {
    /// Indicates whether the process is stopped.
    is_stopped: AtomicBool,

    /// Indicates whether the process's status has changed and has not yet been waited on.
    ///
    /// User programs may use the wait* syscalls to check for changes in
    /// the process's status. This field will be set to `Some(_)` once the
    /// process's status changes and will be set to `None` if the process
    /// has already been waited on.
    wait_status: SpinLock<Option<StopWaitStatus>>,
}

impl StopStatus {
    pub(self) const fn new() -> Self {
        Self {
            is_stopped: AtomicBool::new(false),
            wait_status: SpinLock::new(None),
        }
    }

    /// Stops the process by some signal.
    ///
    /// The return value indicates whether the stop status has changed.
    pub(super) fn stop(&self, signum: SigNum) -> bool {
        // Hold the lock first to avoid race conditions
        let mut wait_status = self.wait_status.lock();

        if self.is_stopped.load(Ordering::Relaxed) {
            false
        } else {
            self.is_stopped.store(true, Ordering::Relaxed);
            *wait_status = Some(StopWaitStatus::Stopped(signum));
            true
        }
    }

    /// Resumes the process.
    ///
    /// The return value indicates whether the stop status has changed.
    pub(super) fn resume(&self) -> bool {
        // Hold the lock first to avoid race conditions
        let mut wait_status = self.wait_status.lock();

        if self.is_stopped.load(Ordering::Relaxed) {
            self.is_stopped.store(false, Ordering::Relaxed);
            *wait_status = Some(StopWaitStatus::Continue);
            true
        } else {
            false
        }
    }

    /// Returns whether the process is stopped.
    pub(super) fn is_stopped(&self) -> bool {
        self.is_stopped.load(Ordering::Relaxed)
    }

    /// Gets and clears the stop status changes for the `wait` syscall.
    pub(super) fn wait(&self, options: WaitOptions) -> Option<StopWaitStatus> {
        let mut wait_status = self.wait_status.lock();

        if options.contains(WaitOptions::WSTOPPED) {
            if let Some(StopWaitStatus::Stopped(_)) = wait_status.as_ref() {
                return wait_status.take();
            }
        }

        if options.contains(WaitOptions::WCONTINUED) {
            if let Some(StopWaitStatus::Continue) = wait_status.as_ref() {
                return wait_status.take();
            }
        }

        None
    }
}

#[derive(Debug)]
pub(super) enum StopWaitStatus {
    // FIXME: A process can also be stopped by ptrace.
    // Extend this enum to support ptrace.
    Stopped(SigNum),
    Continue,
}
