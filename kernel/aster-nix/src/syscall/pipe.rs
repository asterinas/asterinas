// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc},
        pipe::{PipeReader, PipeWriter},
        utils::{Channel, CreationFlags, StatusFlags},
    },
    prelude::*,
    util::write_val_to_user,
};

pub fn sys_pipe2(fds: Vaddr, flags: u32) -> Result<SyscallReturn> {
    debug!("flags: {:?}", flags);

    let (pipe_reader, pipe_writer) = {
        let (producer, consumer) = Channel::new(PIPE_BUF_SIZE).split();

        let status_flags = StatusFlags::from_bits_truncate(flags);

        (
            PipeReader::new(consumer, status_flags)?,
            PipeWriter::new(producer, status_flags)?,
        )
    };

    let fd_flags = if CreationFlags::from_bits_truncate(flags).contains(CreationFlags::O_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let current = current!();
    let mut file_table = current.file_table().lock();

    let pipe_fds = PipeFds {
        reader_fd: file_table.insert(pipe_reader, fd_flags),
        writer_fd: file_table.insert(pipe_writer, fd_flags),
    };
    debug!("pipe_fds: {:?}", pipe_fds);

    if let Err(err) = write_val_to_user(fds, &pipe_fds) {
        file_table.close_file(pipe_fds.reader_fd).unwrap();
        file_table.close_file(pipe_fds.writer_fd).unwrap();
        return Err(err);
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_pipe(fds: Vaddr) -> Result<SyscallReturn> {
    self::sys_pipe2(fds, 0)
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct PipeFds {
    reader_fd: FileDesc,
    writer_fd: FileDesc,
}

const PIPE_BUF_SIZE: usize = 1024 * 1024;
