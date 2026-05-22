// SPDX-License-Identifier: MPL-2.0

use alloc::boxed::Box;

use crate::FuseResult;

/// The completion state of a submitted FUSE request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuseCompletion {
    /// The request completed with a reply payload length.
    Complete(usize),
    /// The request completed with a malformed response.
    MalformedResponse,
    /// The request completed with a remote FUSE error code.
    RemoteError(i32),
}

impl FuseCompletion {
    /// Returns the completed reply payload length.
    pub fn payload_len(self) -> FuseResult<usize> {
        match self {
            Self::Complete(payload_len) => Ok(payload_len),
            Self::MalformedResponse => Err(crate::FuseError::MalformedResponse),
            Self::RemoteError(error) => Err(crate::FuseError::RemoteError(error)),
        }
    }
}

/// The status of a submitted FUSE request.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FuseStatus {
    /// The request has not completed yet.
    Pending,
    /// The request has completed.
    Completed(FuseCompletion),
}

impl FuseStatus {
    /// Returns whether the request is still pending.
    pub fn is_pending(self) -> bool {
        self == Self::Pending
    }

    /// Returns the completion state if the request has completed.
    pub fn has_completed(self) -> Option<FuseCompletion> {
        match self {
            Self::Pending => None,
            Self::Completed(completion) => Some(completion),
        }
    }
}

/// The completion function type for FUSE operations.
///
/// # Invocation context
///
/// Runs in `Taskless` softirq context after the device replies. The callback
/// must not:
///
/// - Sleep, block on a `WaitQueue`, or acquire any sleeping lock.
/// - Acquire any `SpinLock<_, LocalIrqDisabled>` already held by the
///   completion path, especially `FsRequestQueue::inner`.
/// - Drop resources whose `Drop` implementations may sleep.
/// - Allocate unbounded resources or perform long-running work.
pub type FuseCompleteFn = Box<dyn FnOnce(FuseCompletion) + Send>;
