// SPDX-License-Identifier: MPL-2.0

use super::{IoUringOp, completion_from_result};
use crate::io_uring::{IoUringContext, c_types::IoUringSqe, utils::Completion};

pub(super) struct IoUringNopRequest {
    user_data: u64,
    force_async: bool,
}

impl IoUringNopRequest {
    pub(super) fn new(_context: &IoUringContext, sqe: &IoUringSqe, force_async: bool) -> Self {
        Self {
            user_data: sqe.user_data,
            force_async,
        }
    }
}

impl IoUringOp for IoUringNopRequest {
    fn try_execute_nonblock(&self) -> Option<Completion> {
        if self.force_async {
            return None;
        }

        Some(completion_from_result(Ok(0), self.user_data))
    }

    fn execute(&self) -> Completion {
        completion_from_result(Ok(0), self.user_data)
    }
}
