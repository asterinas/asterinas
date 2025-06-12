// SPDX-License-Identifier: MPL-2.0

use crate::process::{signal::sig_num::SigNum, WaitOptions};

pub struct ThreadStatus {
    /// Indicates whether the thread is stopped by a signal.
    // FIXME: Can a thread be stopped by means other than a signal?
    is_stopped: Option<SigNum>,

    /// Indicates whether the thread's status has changed and has not yet been waited on.
    ///
    /// User programs may use the wait* syscalls to check for changes in
    /// the thread's status. This field will be set to true once the
    /// thread's status changes and will be set to false if the thread
    /// has already been waited on.
    has_status_changed: bool,
}

impl ThreadStatus {
    pub(super) const fn new() -> Self {
        Self {
            is_stopped: None,
            has_status_changed: false,
        }
    }

    /// Stops the thread by some signal.
    ///
    /// The return value indicates whether the thread status has changed.
    pub(super) fn stop(&mut self, signum: SigNum) -> bool {
        let status_changed = self.is_stopped.is_none();
        self.has_status_changed |= status_changed;
        self.is_stopped = Some(signum);
        status_changed
    }

    /// Resumes the thread.
    ///
    /// The return value indicates whether the thread status has changed.
    pub(super) fn resume(&mut self) -> bool {
        let status_changed = self.is_stopped.is_some();
        self.has_status_changed |= status_changed;
        self.is_stopped = None;
        status_changed
    }

    /// Returns whether the thread is stopped.
    pub const fn is_stopped(&self) -> bool {
        self.is_stopped.is_some()
    }

    /// Waits on the thread status changes.
    pub fn wait(&mut self, options: WaitOptions) -> Option<ThreadWaitStatus> {
        if !self.has_status_changed {
            return None;
        }

        if options.contains(WaitOptions::WSTOPPED)
            && let Some(signum) = self.is_stopped.as_ref()
        {
            if !options.contains(WaitOptions::WNOWAIT) {
                self.has_status_changed = false;
            }
            return Some(ThreadWaitStatus::Stopped(*signum));
        }

        if options.contains(WaitOptions::WCONTINUED) && self.is_stopped.is_none() {
            if !options.contains(WaitOptions::WNOWAIT) {
                self.has_status_changed = false;
            }
            return Some(ThreadWaitStatus::Continue);
        }

        None
    }
}

pub enum ThreadWaitStatus {
    Stopped(SigNum),
    Continue,
}

impl ThreadWaitStatus {
    pub fn as_u32(&self) -> u32 {
        match self {
            ThreadWaitStatus::Stopped(sig_num) => ((sig_num.as_u8() as u32) << 8) | 0x7f,
            ThreadWaitStatus::Continue => 0xffff,
        }
    }
}
