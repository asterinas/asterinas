// SPDX-License-Identifier: MPL-2.0

mod file;
mod nop;

use ostd::task::Task;

use super::{
    c_types::{IoUringOpcode, IoUringSqe, IoUringSqeFlags},
    io_context::IoUringContext,
};
use crate::{
    fs::file::{FileLike, file_table::FileDesc},
    io_uring::utils::Completion,
    prelude::*,
};

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
        IoUringOpcode::Read => Ok(Arc::new(file::IoUringReadRequest::new(
            context,
            sqe,
            force_async,
        )?)),
        IoUringOpcode::Write => Ok(Arc::new(file::IoUringWriteRequest::new(
            context,
            sqe,
            force_async,
        )?)),
    }
}

fn completion_from_result(result: Result<usize>, user_data: u64) -> Completion {
    let result = match result {
        Ok(value) => i32::try_from(value).unwrap_or(-(Errno::EOVERFLOW as i32)),
        Err(err) => -(err.error() as i32),
    };
    Completion::new(user_data, result, 0)
}

fn check_rw_flags(sqe: &IoUringSqe) -> Result<()> {
    if sqe.rw_flags != 0 {
        return_errno_with_message!(Errno::EINVAL, "SQE read/write flags are unsupported");
    }
    Ok(())
}

fn get_file(fd: i32) -> Result<Arc<dyn FileLike>> {
    let file_desc: FileDesc = fd.try_into()?;
    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();
    let file_table = thread_local.borrow_file_table();
    Ok(file_table.unwrap().read().get_file(file_desc)?.clone())
}

fn check_sqe_flags(sqe: &IoUringSqe) -> Result<IoUringSqeFlags> {
    let flags = IoUringSqeFlags::from_bits(sqe.flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown SQE flags"))?;
    if !flags.difference(IoUringSqeFlags::SUPPORTED).is_empty() {
        return_errno_with_message!(Errno::EINVAL, "SQE flags are unsupported");
    }

    Ok(flags)
}
