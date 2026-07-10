// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use crate::{
    io_uring::register::MAX_REGISTERED_BUFFERS,
    prelude::*,
    util::{IoVec, copy_iovs_and_convert},
};

pub(super) fn resolve_registered_buffers(
    iov_addr: Vaddr,
    iov_count: usize,
) -> Result<Box<[IoVec]>> {
    let task = Task::current().unwrap();
    let thread_local = task.as_thread_local().unwrap();
    let user_space = CurrentUserSpace::new(thread_local);

    let buffers = copy_iovs_and_convert(&user_space, iov_addr, iov_count, |iov, _| Ok(*iov))?;

    if buffers.len() > MAX_REGISTERED_BUFFERS {
        return_errno_with_message!(Errno::EINVAL, "too many io_uring buffers to register");
    }

    Ok(buffers)
}

/// Stores a completion queue entry payload before it is published.
#[derive(Clone, Copy)]
pub(super) struct Completion {
    pub(super) user_data: u64,
    pub(super) res: i32,
    pub(super) flags: u32,
}

impl Completion {
    pub(super) fn new(user_data: u64, res: i32, flags: u32) -> Self {
        Self {
            user_data,
            res,
            flags,
        }
    }

    pub(super) fn with_error(user_data: u64, err: Error) -> Self {
        Self::new(user_data, -(err.error() as i32), 0)
    }
}
