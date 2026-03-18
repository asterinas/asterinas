// SPDX-License-Identifier: MPL-2.0

use super::*;

/// The result of a ptrace-stop.
pub enum PtraceStopResult {
    /// The ptrace-stop is continued by the tracer.
    Continued,
    /// The ptrace-stop is interrupted by `SIGKILL`.
    Interrupted,
    /// The thread is not traced.
    NotTraced(Box<dyn Signal>),
}

/// The signal info of a ptrace-stop.
#[derive(Default)]
pub(super) enum StopSigInfo {
    /// The signal info that has not yet been waited on.
    UnWaited(siginfo_t),
    /// The signal info that has been waited on.
    Waited(siginfo_t),
    /// No ptrace-stop signal info recorded.
    #[default]
    None,
}

impl StopSigInfo {
    /// Records the signal info of a ptrace-stop.
    pub(super) fn stop(&mut self, siginfo: siginfo_t) {
        *self = Self::UnWaited(siginfo);
    }

    /// Clears the ptrace-stop signal info.
    pub(super) fn clear(&mut self) {
        *self = Self::None;
    }

    /// Waits on the ptrace-stop signal info and returns it,
    /// if it has not yet been waited on.
    pub(super) fn wait(&mut self) -> Option<siginfo_t> {
        match *self {
            Self::UnWaited(siginfo) => {
                *self = Self::Waited(siginfo);
                Some(siginfo)
            }
            Self::Waited(_) | Self::None => None,
        }
    }

    /// Returns the ptrace-stop signal info.
    #[expect(dead_code)]
    pub(super) fn get(&self) -> Option<siginfo_t> {
        match self {
            Self::UnWaited(siginfo) | Self::Waited(siginfo) => Some(*siginfo),
            Self::None => None,
        }
    }
}
