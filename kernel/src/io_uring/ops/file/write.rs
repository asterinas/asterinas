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
    util::{IoVec, VmReaderArray},
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

pub(in crate::io_uring::ops) struct IoUringWriteVRequest {
    user_data: u64,
    force_async: bool,
    file: Arc<dyn FileLike>,
    offset: u64,
    io_vec_ptr: Vaddr,
    io_vec_count: usize,
}

impl IoUringWriteVRequest {
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

    fn write_vectored<'a>(
        &self,
        user_space: &'a CurrentUserSpace<'a>,
        iov_addr: Vaddr,
        iov_count: usize,
    ) -> Result<usize> {
        let mut reader_array = VmReaderArray::from_user_io_vecs(user_space, iov_addr, iov_count)?;

        if self.offset == CURRENT_FILE_OFFSET {
            self.writev_current_offset(&mut reader_array)
        } else {
            if reader_array.readers_mut().is_empty() {
                let empty = [0u8; 0];
                let mut reader = VmReader::from(empty.as_slice()).to_fallible();
                self.file.write_at(self.offset as usize, &mut reader)?;
                return Ok(0);
            }

            self.writev_at_offset(&mut reader_array)
        }
    }

    fn writev_current_offset(&self, reader_array: &mut VmReaderArray<'_>) -> Result<usize> {
        let mut total_len = 0;

        for reader in reader_array.readers_mut() {
            debug_assert!(reader.has_remain());

            match self.file.write(reader) {
                Ok(write_len) => total_len += write_len,
                Err(_) if total_len > 0 => break,
                Err(err) => return Err(err),
            }
            if reader.has_remain() {
                break;
            }
        }

        Ok(total_len)
    }

    fn writev_at_offset(&self, reader_array: &mut VmReaderArray<'_>) -> Result<usize> {
        let mut total_len = 0;
        let mut cur_offset = self.offset as usize;

        for reader in reader_array.readers_mut() {
            debug_assert!(reader.has_remain());

            match self.file.write_at(cur_offset, reader) {
                Ok(write_len) => {
                    total_len += write_len;
                    cur_offset += write_len;
                }
                Err(_) if total_len > 0 => break,
                Err(err) => return Err(err),
            }
            if reader.has_remain() {
                break;
            }
        }

        Ok(total_len)
    }
}

impl IoUringOp for IoUringWriteVRequest {
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

            let write_len = self.write_vectored(&user_space, self.io_vec_ptr, self.io_vec_count)?;

            if write_len > 0 {
                fs::vfs::notify::on_modify(&self.file);
            }
            Ok(write_len)
        })();

        completion_from_result(result, self.user_data)
    }
}
