// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc},
        pipe,
        utils::CreationFlags,
    },
    prelude::*,
};

pub fn sys_pipe2(fds: Vaddr, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("flags: {:?}", flags);

    let (pipe_reader, pipe_writer) = pipe::new_pair()?;

    let fd_flags = if CreationFlags::from_bits_truncate(flags).contains(CreationFlags::O_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let mut file_table = ctx.posix_thread.file_table().lock();

    let pipe_fds = PipeFds {
        reader_fd: file_table.insert(pipe_reader, fd_flags),
        writer_fd: file_table.insert(pipe_writer, fd_flags),
    };
    debug!("pipe_fds: {:?}", pipe_fds);

    if let Err(err) = ctx.user_space().write_val(fds, &pipe_fds) {
        file_table.close_file(pipe_fds.reader_fd).unwrap();
        file_table.close_file(pipe_fds.writer_fd).unwrap();
        return Err(err);
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_pipe(fds: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_pipe2(fds, 0, ctx)
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
struct PipeFds {
    reader_fd: FileDesc,
    writer_fd: FileDesc,
}
