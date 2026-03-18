// SPDX-License-Identifier: MPL-2.0

//! Ptrace utilities for POSIX threads.

use crate::{
    prelude::*,
    process::{
        WaitOptions,
        signal::{DequeuedSignal, sig_num::SigNum, signals::Signal},
    },
};

/// The requests that can continue a stopped tracee.
pub enum PtraceContRequest {
    Continue(Option<SigNum>),
    #[expect(dead_code)]
    SingleStep(Option<SigNum>),
    #[expect(dead_code)]
    Syscall(Option<SigNum>),
}

impl PtraceContRequest {
    pub(super) fn sig_num(&self) -> Option<SigNum> {
        match self {
            Self::Continue(Some(sig_num))
            | Self::SingleStep(Some(sig_num))
            | Self::Syscall(Some(sig_num)) => Some(*sig_num),
            _ => None,
        }
    }
}

/// The result of a ptrace-stop.
pub enum PtraceStopResult {
    /// The ptrace-stop is continued by the tracer,
    /// or ends because the tracer exits or detaches.
    Continued(Option<DequeuedSignal>),
    /// The ptrace-stop is interrupted by `SIGKILL`.
    Interrupted,
    /// The thread is not traced, returning the stop signal back.
    NotTraced(DequeuedSignal),
}

/// The signal associated with a ptrace-stop and its later signal delivery.
#[derive(Default)]
pub(super) enum StopDeliverySignal {
    /// The signal that has not yet been reported through `wait`.
    Pending(DequeuedSignal),
    /// The signal that has been reported through `wait`.
    Consumed(DequeuedSignal),
    /// The signal that is injected by the tracer.
    Injected(DequeuedSignal),
    /// No ptrace-stop signal is recorded.
    #[default]
    Empty,
}

impl StopDeliverySignal {
    /// Records the signal associated with a ptrace-stop.
    pub(super) fn stop(&mut self, signal: DequeuedSignal) {
        *self = Self::Pending(signal);
    }

    /// Clears and returns the signal associated with a ptrace-stop,
    /// unless it has already been consumed by `wait`.
    pub(super) fn clear(&mut self) -> Option<DequeuedSignal> {
        let this = core::mem::replace(self, Self::Empty);

        match this {
            Self::Pending(signal) | Self::Injected(signal) => Some(signal),
            Self::Consumed(_) | Self::Empty => None,
        }
    }

    /// Returns the signal associated with a ptrace-stop,
    /// if it has not yet been reported through `wait`.
    pub(super) fn wait(&mut self, options: WaitOptions) -> Option<&dyn Signal> {
        let this = core::mem::replace(self, Self::Empty);

        match this {
            Self::Pending(signal) => {
                if !options.contains(WaitOptions::WNOWAIT) {
                    *self = Self::Consumed(signal);
                } else {
                    *self = Self::Pending(signal);
                }
                Some(self.get().unwrap())
            }
            Self::Consumed(signal) => {
                *self = Self::Consumed(signal);
                None
            }
            Self::Injected(_) => unreachable!(),
            Self::Empty => None,
        }
    }

    /// Injects a signal by the tracer.
    pub(super) fn inject(&mut self, new_signal: Box<dyn Signal>) {
        let this = core::mem::replace(self, Self::Empty);

        let mut signal = match this {
            Self::Pending(signal) | Self::Consumed(signal) => signal,
            Self::Injected(_) | Self::Empty => unreachable!(),
        };

        signal.set_signal(new_signal);
        *self = Self::Injected(signal);
    }

    /// Returns the signal associated with a ptrace-stop,
    /// but does not change the state.
    fn get(&self) -> Option<&dyn Signal> {
        match self {
            Self::Pending(signal) | Self::Consumed(signal) | Self::Injected(signal) => {
                Some(signal.signal())
            }
            Self::Empty => None,
        }
    }
}
