// SPDX-License-Identifier: MPL-2.0

use crate::{
    prelude::*,
    process::{
        Process,
        posix_thread::PosixThread,
        signal::{
            SigNum,
            constants::SIGKILL,
            sig_mask::{SigMask, SigSet},
            signals::Signal,
        },
    },
};

/// A signal dequeued from either the thread or the process queue,
/// carrying its origin so it can be correctly re-enqueued later.
pub enum DequeuedSignal {
    FromProcess(Box<dyn Signal>),
    FromThread(Box<dyn Signal>),
}

impl DequeuedSignal {
    /// Consumes the dequeued signal and returns the inner signal.
    pub fn unwrap(self) -> Box<dyn Signal> {
        match self {
            Self::FromProcess(signal) | Self::FromThread(signal) => signal,
        }
    }

    /// Returns a reference to the inner signal.
    pub(in crate::process) fn signal(&self) -> &dyn Signal {
        match self {
            Self::FromProcess(signal) | Self::FromThread(signal) => signal.as_ref(),
        }
    }

    /// Returns the signal number of the inner signal.
    pub(in crate::process) fn num(&self) -> SigNum {
        self.signal().num()
    }

    /// Replaces the inner signal with `new_signal`, preserving the origin.
    pub(in crate::process) fn set_signal(&mut self, new_signal: Box<dyn Signal>) {
        match self {
            Self::FromProcess(signal) | Self::FromThread(signal) => *signal = new_signal,
        }
    }
}

/// Trait for handling pending signals.
pub trait HandlePendingSignal {
    /// Returns the thread's pending signal set.
    ///
    /// This includes signals that are currently blocked or ignored.
    fn pending_signals(&self) -> SigSet;

    /// Returns if there are pending signals that are neither blocked nor ignored.
    ///
    /// Note that ignored but not blocked signals may be dequeued silently.
    fn has_pending(&self) -> bool;

    /// Returns if a SIGKILL signal is pending.
    fn has_pending_sigkill(&self) -> bool;

    /// Dequeues the next pending signal that is not masked by `mask`.
    ///
    /// Returns `None` if no such signal is available.
    fn dequeue_signal(&self, mask: &SigMask) -> Option<DequeuedSignal>;
}

impl HandlePendingSignal for Context<'_> {
    fn pending_signals(&self) -> SigSet {
        self.posix_thread.sig_queues().sig_pending() | self.process.sig_queues().sig_pending()
    }

    fn has_pending(&self) -> bool {
        let posix_thread = self.posix_thread;
        let process = self.process.as_ref();
        has_pending_signal(posix_thread, process)
    }

    fn has_pending_sigkill(&self) -> bool {
        self.posix_thread.sig_queues().has_pending_signal(SIGKILL)
            || self.process.sig_queues().has_pending_signal(SIGKILL)
    }

    fn dequeue_signal(&self, mask: &SigMask) -> Option<DequeuedSignal> {
        self.posix_thread
            .sig_queues()
            .dequeue(mask)
            .map(DequeuedSignal::FromThread)
            .or_else(|| {
                self.process
                    .sig_queues()
                    .dequeue(mask)
                    .map(DequeuedSignal::FromProcess)
            })
    }
}

impl HandlePendingSignal for PosixThread {
    fn pending_signals(&self) -> SigSet {
        self.sig_queues().sig_pending() | self.process().sig_queues().sig_pending()
    }

    fn has_pending(&self) -> bool {
        let process = self.process();
        has_pending_signal(self, process.as_ref())
    }

    fn has_pending_sigkill(&self) -> bool {
        self.sig_queues().has_pending_signal(SIGKILL)
            || self.process().sig_queues().has_pending_signal(SIGKILL)
    }

    fn dequeue_signal(&self, mask: &SigMask) -> Option<DequeuedSignal> {
        self.sig_queues()
            .dequeue(mask)
            .map(DequeuedSignal::FromThread)
            .or_else(|| {
                self.process()
                    .sig_queues()
                    .dequeue(mask)
                    .map(DequeuedSignal::FromProcess)
            })
    }
}

fn has_pending_signal(posix_thread: &PosixThread, process: &Process) -> bool {
    // Fast path: No signals are pending.
    if posix_thread.sig_queues().is_empty() && process.sig_queues().is_empty() {
        return false;
    }

    // Slow path: Some signals are pending.
    let sig_dispositions = process.sig_dispositions().lock();
    let sig_dispositions = sig_dispositions.lock();
    let blocked = posix_thread.sig_mask();

    posix_thread
        .sig_queues()
        .has_pending(blocked, &sig_dispositions)
        || process.sig_queues().has_pending(blocked, &sig_dispositions)
}
