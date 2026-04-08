// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        file::{
            CreationFlags, StatusFlags,
            file_table::{FdFlags, RawFileDesc},
        },
        pipe,
    },
    prelude::*,
};

pub fn sys_pipe2(fds: Vaddr, flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("flags: {:?}", flags);

    const VALID_FLAGS: u32 = CreationFlags::O_CLOEXEC.bits()
        | StatusFlags::O_NONBLOCK.bits()
        | StatusFlags::O_DIRECT.bits();
    if flags & !VALID_FLAGS != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid pipe flags");
    }

    let status_flags = StatusFlags::from_bits_truncate(flags);
    let (pipe_reader, pipe_writer) = pipe::new_file_pair(status_flags)?;

    let creation_flags = CreationFlags::from_bits_truncate(flags);
    let fd_flags = if creation_flags.contains(CreationFlags::O_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let file_table = ctx.thread_local.borrow_file_table();
    let mut file_table_locked = file_table.unwrap().write();

    let reader_fd = file_table_locked.insert(pipe_reader, fd_flags);
    let writer_fd = file_table_locked.insert(pipe_writer, fd_flags);
    let pipe_fds = PipeFds {
        reader_raw_fd: reader_fd.into(),
        writer_raw_fd: writer_fd.into(),
    };
    debug!("pipe_fds: {:?}", pipe_fds);

    if let Err(err) = ctx.user_space().write_val(fds, &pipe_fds) {
        file_table_locked.close_file(reader_fd).unwrap();
        file_table_locked.close_file(writer_fd).unwrap();
        return Err(err.into());
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_pipe(fds: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_pipe2(fds, 0, ctx)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct PipeFds {
    reader_raw_fd: RawFileDesc,
    writer_raw_fd: RawFileDesc,
}
