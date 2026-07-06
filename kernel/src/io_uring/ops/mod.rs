// SPDX-License-Identifier: MPL-2.0

mod nop;

use super::{
    c_types::{IoUringOpcode, IoUringSqe, IoUringSqeFlags},
    io_context::IoUringContext,
};
use crate::{io_uring::utils::Completion, prelude::*};

pub(super) trait IoUringOp: Send + Sync {
    // Normal operation for io_uring is to try and issue an sqe as
    // non-blocking first, and if that fails, execute it in async manner.
    fn try_execute_nonblock(&self) -> Option<Completion>;

    fn execute(&self) -> Completion;
}

pub(super) fn build_op_request(
    context: &IoUringContext,
    sqe: &IoUringSqe,
) -> Result<Arc<dyn IoUringOp>> {
    let flags = check_sqe_flags(sqe)?;
    let force_async = flags.contains(IoUringSqeFlags::ASYNC);

    match IoUringOpcode::try_from(sqe.opcode)? {
        IoUringOpcode::Nop => Ok(Arc::new(nop::IoUringNopRequest::new(
            context,
            sqe,
            force_async,
        ))),
    }
}

fn completion_from_result(result: Result<usize>, user_data: u64) -> Completion {
    let result = match result {
        Ok(value) => i32::try_from(value).unwrap_or(-(Errno::EOVERFLOW as i32)),
        Err(err) => -(err.error() as i32),
    };
    Completion::new(user_data, result, 0)
}

fn check_sqe_flags(sqe: &IoUringSqe) -> Result<IoUringSqeFlags> {
    let flags = IoUringSqeFlags::from_bits(sqe.flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown SQE flags"))?;
    if !flags.difference(IoUringSqeFlags::SUPPORTED).is_empty() {
        return_errno_with_message!(Errno::EINVAL, "SQE flags are unsupported");
    }

    Ok(flags)
}
