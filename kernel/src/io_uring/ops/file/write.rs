// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use super::CURRENT_FILE_OFFSET;
use crate::{
    fs::{self, file::FileLike},
    io_uring::{
        c_types::{IoUringOpcode, IoUringSqe},
        io_context::IoUringContext,
        ops::{IoUringOp, check_rw_flags, completion_from_result, get_file},
        utils::Completion,
    },
    prelude::*,
    util::IoVec,
};

pub(in crate::io_uring::ops) struct IoUringWriteRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    offset: u64,
    buffer: IoVec,
}

impl IoUringWriteRequest {
    pub(in crate::io_uring::ops) fn new(
        context: &IoUringContext,
        sqe: &IoUringSqe,
        force_async: bool,
    ) -> Result<Self> {
        check_rw_flags(sqe)?;

        let buffer = match IoUringOpcode::try_from(sqe.opcode)? {
            IoUringOpcode::Write => IoVec {
                base: sqe.addr as Vaddr,
                len: sqe.len as usize,
            },
            IoUringOpcode::WriteFixed => {
                context.get_registered_buffer(sqe.buf_index, sqe.addr as Vaddr, sqe.len as usize)?
            }
            _ => {
                return_errno_with_message!(Errno::EINVAL, "SQE opcode is not a write operation")
            }
        };

        Ok(Self {
            user_data: sqe.user_data,
            force_async,
            file: get_file(sqe.fd)?,
            offset: sqe.off,
            buffer,
        })
    }
}

impl IoUringOp for IoUringWriteRequest {
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
            let mut reader = user_space.reader(self.buffer.base, self.buffer.len)?;

            let write_len = if self.offset == CURRENT_FILE_OFFSET {
                self.file.write(&mut reader)?
            } else {
                self.file.write_at(self.offset as usize, &mut reader)?
            };

            if write_len > 0 {
                fs::vfs::notify::on_modify(&self.file);
            }
            Ok(write_len)
        })();

        completion_from_result(result, self.user_data)
    }
}
