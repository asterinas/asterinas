// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_PIPE2};
use crate::{
    fs::{
        file_table::{FdFlags, FileDescripter},
        pipe::{PipeReader, PipeWriter},
        utils::{Channel, CreationFlags, StatusFlags},
    },
    log_syscall_entry,
    prelude::*,
    util::{read_val_from_user, write_val_to_user},
};

pub fn sys_pipe2(fds: Vaddr, flags: u32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_PIPE2);
    debug!("flags: {:?}", flags);

    let mut pipe_fds = read_val_from_user::<PipeFds>(fds)?;
    let (reader, writer) = {
        let (producer, consumer) = Channel::with_capacity_and_flags(
            PIPE_BUF_SIZE,
            StatusFlags::from_bits_truncate(flags),
        )?
        .split();
        (PipeReader::new(consumer), PipeWriter::new(producer))
    };
    let pipe_reader = Arc::new(reader);
    let pipe_writer = Arc::new(writer);
    let fd_flags = if CreationFlags::from_bits_truncate(flags).contains(CreationFlags::O_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let current = current!();
    let mut file_table = current.file_table().lock();
    pipe_fds.reader_fd = file_table.insert(pipe_reader, fd_flags);
    pipe_fds.writer_fd = file_table.insert(pipe_writer, fd_flags);
    debug!("pipe_fds: {:?}", pipe_fds);
    write_val_to_user(fds, &pipe_fds)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_pipe(fds: Vaddr) -> Result<SyscallReturn> {
    self::sys_pipe2(fds, 0)
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct PipeFds {
    reader_fd: FileDescripter,
    writer_fd: FileDescripter,
}

const PIPE_BUF_SIZE: usize = 1024 * 1024;
