// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc},
        inode_handle::InodeHandle,
        utils::{c_flock, FileRange, RangeLockBuilder, RangeLockType, StatusFlags},
    },
    prelude::*,
    util::read_val_from_user,
};

pub fn sys_fcntl(fd: FileDesc, cmd: i32, arg: u64) -> Result<SyscallReturn> {
    let fcntl_cmd = FcntlCmd::try_from(cmd)?;
    debug!("fd = {}, cmd = {:?}, arg = {}", fd, fcntl_cmd, arg);

    match fcntl_cmd {
        FcntlCmd::F_DUPFD => handle_dupfd(fd, arg, FdFlags::empty()),
        FcntlCmd::F_DUPFD_CLOEXEC => handle_dupfd(fd, arg, FdFlags::CLOEXEC),
        FcntlCmd::F_GETFD => handle_getfd(fd),
        FcntlCmd::F_SETFD => handle_setfd(fd, arg),
        FcntlCmd::F_GETFL => handle_getfl(fd),
        FcntlCmd::F_SETFL => handle_setfl(fd, arg),
        FcntlCmd::F_GETLK => handle_getlk(fd, arg),
        FcntlCmd::F_SETLK => handle_setlk(fd, arg, true),
        FcntlCmd::F_SETLKW => handle_setlk(fd, arg, false),
    }
}

fn handle_dupfd(fd: FileDesc, arg: u64, flags: FdFlags) -> Result<SyscallReturn> {
    let current = current!();
    let mut file_table = current.file_table().lock();
    let new_fd = file_table.dup(fd, arg as FileDesc, flags)?;
    Ok(SyscallReturn::Return(new_fd as _))
}

fn handle_getfd(fd: FileDesc) -> Result<SyscallReturn> {
    let current = current!();
    let file_table = current.file_table().lock();
    let entry = file_table.get_entry(fd)?;
    let fd_flags = entry.flags();
    Ok(SyscallReturn::Return(fd_flags.bits() as _))
}

fn handle_setfd(fd: FileDesc, arg: u64) -> Result<SyscallReturn> {
    let flags = if arg > u8::MAX.into() {
        return_errno_with_message!(Errno::EINVAL, "invalid fd flags");
    } else {
        FdFlags::from_bits(arg as u8).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?
    };
    let current = current!();
    let file_table = current.file_table().lock();
    let entry = file_table.get_entry(fd)?;
    entry.set_flags(flags);
    Ok(SyscallReturn::Return(0))
}

fn handle_getfl(fd: FileDesc) -> Result<SyscallReturn> {
    let current = current!();
    let file = {
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let status_flags = file.status_flags();
    let access_mode = file.access_mode();
    Ok(SyscallReturn::Return(
        (status_flags.bits() | access_mode as u32) as _,
    ))
}

fn handle_setfl(fd: FileDesc, arg: u64) -> Result<SyscallReturn> {
    let current = current!();
    let file = {
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let valid_flags_mask = StatusFlags::O_APPEND
        | StatusFlags::O_ASYNC
        | StatusFlags::O_DIRECT
        | StatusFlags::O_NOATIME
        | StatusFlags::O_NONBLOCK;
    let mut status_flags = file.status_flags();
    status_flags.remove(valid_flags_mask);
    status_flags.insert(StatusFlags::from_bits_truncate(arg as _) & valid_flags_mask);
    file.set_status_flags(status_flags)?;
    Ok(SyscallReturn::Return(0))
}

fn handle_getlk(fd: FileDesc, arg: u64) -> Result<SyscallReturn> {
    let current = current!();
    let file = {
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let lock_mut_ptr = arg as Vaddr;
    let mut lock_mut_c = read_val_from_user::<c_flock>(lock_mut_ptr)?;
    let lock_type = RangeLockType::from_u16(lock_mut_c.l_type)?;
    if lock_type == RangeLockType::F_UNLCK {
        return_errno_with_message!(Errno::EINVAL, "invalid flock type for getlk");
    }
    let mut lock = RangeLockBuilder::new()
        .type_(lock_type)
        .range(FileRange::from_c_flock_and_file(&lock_mut_c, file.clone())?)
        .build()?;
    let inode_file = file
        .downcast_ref::<InodeHandle>()
        .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
    inode_file.test_advisory_lock(&mut lock)?;
    lock_mut_c.copy_from_range_lock(&lock);
    Ok(SyscallReturn::Return(0))
}

fn handle_setlk(fd: FileDesc, arg: u64, is_nonblocking: bool) -> Result<SyscallReturn> {
    let current = current!();
    let file = {
        let file_table = current.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let lock_mut_ptr = arg as Vaddr;
    let lock_mut_c = read_val_from_user::<c_flock>(lock_mut_ptr)?;
    let lock_type = RangeLockType::from_u16(lock_mut_c.l_type)?;
    let lock = RangeLockBuilder::new()
        .type_(lock_type)
        .range(FileRange::from_c_flock_and_file(&lock_mut_c, file.clone())?)
        .build()?;
    let inode_file = file
        .downcast_ref::<InodeHandle>()
        .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
    inode_file.set_advisory_lock(&lock, is_nonblocking)?;
    Ok(SyscallReturn::Return(0))
}

#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
enum FcntlCmd {
    F_DUPFD = 0,
    F_GETFD = 1,
    F_SETFD = 2,
    F_GETFL = 3,
    F_SETFL = 4,
    F_SETLK = 6,
    F_SETLKW = 7,
    F_GETLK = 8,
    F_DUPFD_CLOEXEC = 1030,
}
