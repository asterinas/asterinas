// SPDX-License-Identifier: MPL-2.0

use ostd::task::Task;

use super::CURRENT_FILE_OFFSET;
use crate::{
    fs::{self, file::FileLike},
    io_uring::{
        c_types::{IoUringOpcode, IoUringSqe},
        io_context::IoUringContext,
        ops::{IoUringOp, completion_from_result, get_file},
        utils::Completion,
    },
    prelude::*,
    util::{IoVec, VmWriterArray},
};

pub(in crate::io_uring::ops) struct IoUringReadRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    offset: u64,
    buffer: IoVec,
}

impl IoUringReadRequest {
    pub(in crate::io_uring::ops) fn new(
        context: &IoUringContext,
        sqe: &IoUringSqe,
        force_async: bool,
    ) -> Result<Self> {
        let buffer = match IoUringOpcode::try_from(sqe.opcode)? {
            IoUringOpcode::Read => IoVec {
                base: sqe.addr as Vaddr,
                len: sqe.len as usize,
            },
            IoUringOpcode::ReadFixed => {
                context.get_registered_buffer(sqe.buf_index, sqe.addr as Vaddr, sqe.len as usize)?
            }
            _ => {
                return_errno_with_message!(Errno::EINVAL, "SQE opcode is not a read operation")
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

impl IoUringOp for IoUringReadRequest {
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
            let mut writer = user_space.writer(self.buffer.base, self.buffer.len)?;

            let read_len = if self.offset == CURRENT_FILE_OFFSET {
                self.file.read(&mut writer)?
            } else {
                self.file.read_at(self.offset as usize, &mut writer)?
            };

            if read_len > 0 {
                fs::vfs::notify::on_access(&self.file);
            }
            Ok(read_len)
        })();

        completion_from_result(result, self.user_data)
    }
}

pub(in crate::io_uring::ops) struct IoUringReadVRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    offset: u64,
    io_vec_ptr: Vaddr,
    io_vec_count: usize,
}

impl IoUringReadVRequest {
    pub(in crate::io_uring::ops) fn new(
        _context: &IoUringContext,
        sqe: &IoUringSqe,
        force_async: bool,
    ) -> Result<Self> {
        Ok(Self {
            user_data: sqe.user_data,
            force_async,
            file: get_file(sqe.fd)?,
            offset: sqe.off,
            io_vec_ptr: sqe.addr as Vaddr,
            io_vec_count: sqe.len as usize,
        })
    }

    fn read_vectored<'a>(
        &self,
        user_space: &'a CurrentUserSpace<'a>,
        iov_addr: Vaddr,
        iov_count: usize,
    ) -> Result<usize> {
        let mut writer_array = VmWriterArray::from_user_io_vecs(user_space, iov_addr, iov_count)?;

        if self.offset == CURRENT_FILE_OFFSET {
            self.readv_current_offset(&mut writer_array)
        } else {
            if writer_array.writers_mut().is_empty() {
                let mut empty = [0u8; 0];
                let mut writer = VmWriter::from(empty.as_mut_slice()).to_fallible();
                self.file.read_at(self.offset as usize, &mut writer)?;
                return Ok(0);
            }

            self.readv_at_offset(&mut writer_array)
        }
    }

    fn readv_current_offset(&self, writer_array: &mut VmWriterArray<'_>) -> Result<usize> {
        let mut total_len = 0;

        for writer in writer_array.writers_mut() {
            debug_assert!(writer.has_avail());

            match self.file.read(writer) {
                Ok(read_len) => total_len += read_len,
                Err(_) if total_len > 0 => break,
                Err(err) => return Err(err),
            }
            if writer.has_avail() {
                break;
            }
        }

        Ok(total_len)
    }

    fn readv_at_offset(&self, writer_array: &mut VmWriterArray<'_>) -> Result<usize> {
        let mut total_len = 0;
        let mut cur_offset = self.offset as usize;

        for writer in writer_array.writers_mut() {
            debug_assert!(writer.has_avail());

            match self.file.read_at(cur_offset, writer) {
                Ok(read_len) => {
                    total_len += read_len;
                    cur_offset += read_len;
                }
                Err(_) if total_len > 0 => break,
                Err(err) => return Err(err),
            }
            if writer.has_avail() {
                break;
            }
        }

        Ok(total_len)
    }
}

impl IoUringOp for IoUringReadVRequest {
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

            let read_len = self.read_vectored(&user_space, self.io_vec_ptr, self.io_vec_count)?;

            if read_len > 0 {
                fs::vfs::notify::on_access(&self.file);
            }
            Ok(read_len)
        })();

        completion_from_result(result, self.user_data)
    }
}
