// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use crate::{
    fs::file::FileLike,
    io_uring::{
        c_types::IoUringSqe,
        io_context::IoUringContext,
        ops::{IoUringOp, completion_from_result, get_file},
        utils::Completion,
    },
    net::socket::util::SendRecvFlags,
    prelude::*,
    util::IoVec,
};

pub(in crate::io_uring::ops) struct IoUringRecvRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    buffer: IoVec,
    flags: SendRecvFlags,
}

impl IoUringRecvRequest {
    pub(in crate::io_uring::ops) fn new(
        _context: &IoUringContext,
        sqe: &IoUringSqe,
        force_async: bool,
    ) -> Result<Self> {
        Ok(Self {
            user_data: sqe.user_data,
            force_async,
            file: get_file(sqe.fd)?,
            buffer: IoVec {
                base: sqe.addr as Vaddr,
                len: sqe.len as usize,
            },
            flags: SendRecvFlags::from_bits_truncate(sqe.op_flags as i32),
        })
    }
}

impl IoUringOp for IoUringRecvRequest {
    fn try_execute_nonblock(&self) -> Option<Completion> {
        if self.force_async {
            return None;
        }

        None
    }

    fn execute(&self) -> Completion {
        let result = (|| {
            let task = Task::current().unwrap();
            let thread_local = task.as_thread_local().unwrap();
            let user_space = CurrentUserSpace::new(thread_local);
            let socket = self.file.as_socket_or_err()?;
            let mut writer = user_space.writer(self.buffer.base, self.buffer.len)?;

            socket.recvmsg(&mut writer, self.flags).map(|(len, _)| len)
        })();

        completion_from_result(result, self.user_data)
    }
}
