// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_handle::FileLike,
        file_table::{FdFlags, FileDesc},
        inode_handle::InodeHandle,
        utils::{
            FileRange, RangeLockItem, RangeLockItemBuilder, RangeLockType, StatusFlags, OFFSET_MAX,
        },
    },
    prelude::*,
    process::{process_table, Pid},
};

pub fn sys_fcntl(fd: FileDesc, cmd: i32, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let fcntl_cmd = FcntlCmd::try_from(cmd)?;
    debug!("fd = {}, cmd = {:?}, arg = {}", fd, fcntl_cmd, arg);
    match fcntl_cmd {
        FcntlCmd::F_DUPFD => handle_dupfd(fd, arg, FdFlags::empty(), ctx),
        FcntlCmd::F_DUPFD_CLOEXEC => handle_dupfd(fd, arg, FdFlags::CLOEXEC, ctx),
        FcntlCmd::F_GETFD => handle_getfd(fd, ctx),
        FcntlCmd::F_SETFD => handle_setfd(fd, arg, ctx),
        FcntlCmd::F_GETFL => handle_getfl(fd, ctx),
        FcntlCmd::F_SETFL => handle_setfl(fd, arg, ctx),
        FcntlCmd::F_GETLK => handle_getlk(fd, arg, ctx),
        FcntlCmd::F_SETLK => handle_setlk(fd, arg, true, ctx),
        FcntlCmd::F_SETLKW => handle_setlk(fd, arg, false, ctx),
        FcntlCmd::F_GETOWN => handle_getown(fd, ctx),
        FcntlCmd::F_SETOWN => handle_setown(fd, arg, ctx),
    }
}

fn handle_dupfd(fd: FileDesc, arg: u64, flags: FdFlags, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.posix_thread.file_table().lock();
    let new_fd = file_table.dup(fd, arg as FileDesc, flags)?;
    Ok(SyscallReturn::Return(new_fd as _))
}

fn handle_getfd(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    let file_table = ctx.posix_thread.file_table().lock();
    let entry = file_table.get_entry(fd)?;
    let fd_flags = entry.flags();
    Ok(SyscallReturn::Return(fd_flags.bits() as _))
}

fn handle_setfd(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let flags = if arg > u64::from(u8::MAX) {
        return_errno_with_message!(Errno::EINVAL, "invalid fd flags");
    } else {
        FdFlags::from_bits(arg as u8).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?
    };
    let file_table = ctx.posix_thread.file_table().lock();
    let entry = file_table.get_entry(fd)?;
    entry.set_flags(flags);
    Ok(SyscallReturn::Return(0))
}

fn handle_getfl(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let status_flags = file.status_flags();
    let access_mode = file.access_mode();
    Ok(SyscallReturn::Return(
        (status_flags.bits() | access_mode as u32) as _,
    ))
}

fn handle_setfl(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
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

fn handle_getlk(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let lock_mut_ptr = arg as Vaddr;
    let mut lock_mut_c = ctx.user_space().read_val::<c_flock>(lock_mut_ptr)?;
    let lock_type = RangeLockType::try_from(lock_mut_c.l_type)?;
    if lock_type == RangeLockType::Unlock {
        return_errno_with_message!(Errno::EINVAL, "invalid flock type for getlk");
    }
    let mut lock = RangeLockItemBuilder::new()
        .type_(lock_type)
        .range(from_c_flock_and_file(&lock_mut_c, file.clone())?)
        .build()?;
    let inode_file = file
        .downcast_ref::<InodeHandle>()
        .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
    lock = inode_file.test_range_lock(lock)?;
    lock_mut_c.copy_from_range_lock(&lock);
    ctx.user_space().write_val(lock_mut_ptr, &lock_mut_c)?;
    Ok(SyscallReturn::Return(0))
}

fn handle_setlk(
    fd: FileDesc,
    arg: u64,
    is_nonblocking: bool,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(fd)?.clone()
    };
    let lock_mut_ptr = arg as Vaddr;
    let lock_mut_c = ctx.user_space().read_val::<c_flock>(lock_mut_ptr)?;
    let lock_type = RangeLockType::try_from(lock_mut_c.l_type)?;
    let lock = RangeLockItemBuilder::new()
        .type_(lock_type)
        .range(from_c_flock_and_file(&lock_mut_c, file.clone())?)
        .build()?;
    let inode_file = file
        .downcast_ref::<InodeHandle>()
        .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
    inode_file.set_range_lock(&lock, is_nonblocking)?;
    Ok(SyscallReturn::Return(0))
}

fn handle_getown(fd: FileDesc, ctx: &Context) -> Result<SyscallReturn> {
    let file_table = ctx.posix_thread.file_table().lock();
    let file_entry = file_table.get_entry(fd)?;
    let pid = file_entry.owner().unwrap_or(0);
    Ok(SyscallReturn::Return(pid as _))
}

fn handle_setown(fd: FileDesc, arg: u64, ctx: &Context) -> Result<SyscallReturn> {
    // A process ID is specified as a positive value; a process group ID is specified as a negative value.
    let abs_arg = (arg as i32).unsigned_abs();
    if abs_arg > i32::MAX as u32 {
        return_errno_with_message!(Errno::EINVAL, "process (group) id overflowed");
    }
    let pid = Pid::try_from(abs_arg)
        .map_err(|_| Error::with_message(Errno::EINVAL, "invalid process (group) id"))?;

    let owner_process = if pid == 0 {
        None
    } else {
        Some(process_table::get_process(pid).ok_or(Error::with_message(
            Errno::ESRCH,
            "cannot set_owner with an invalid pid",
        ))?)
    };

    let mut file_table = ctx.posix_thread.file_table().lock();
    let file_entry = file_table.get_entry_mut(fd)?;
    file_entry.set_owner(owner_process.as_ref())?;
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
    F_GETLK = 5,
    F_SETLK = 6,
    F_SETLKW = 7,
    F_SETOWN = 8,
    F_GETOWN = 9,
    F_DUPFD_CLOEXEC = 1030,
}

#[allow(non_camel_case_types)]
pub type off_t = i64;

#[allow(non_camel_case_types)]
#[derive(Debug, Copy, Clone, TryFromInt)]
#[repr(u16)]
pub enum RangeLockWhence {
    SEEK_SET = 0,
    SEEK_CUR = 1,
    SEEK_END = 2,
}

/// C struct for a file range lock in Libc
#[repr(C)]
#[derive(Debug, Copy, Clone, Pod)]
pub struct c_flock {
    /// Type of lock: F_RDLCK, F_WRLCK, or F_UNLCK
    pub l_type: u16,
    /// Where `l_start' is relative to
    pub l_whence: u16,
    /// Offset where the lock begins
    pub l_start: off_t,
    /// Size of the locked area, 0 means until EOF
    pub l_len: off_t,
    /// Process holding the lock
    pub l_pid: Pid,
}

impl c_flock {
    pub fn copy_from_range_lock(&mut self, lock: &RangeLockItem) {
        self.l_type = lock.type_() as u16;
        if RangeLockType::Unlock != lock.type_() {
            self.l_whence = RangeLockWhence::SEEK_SET as u16;
            self.l_start = lock.start() as off_t;
            self.l_len = if lock.end() == OFFSET_MAX {
                0
            } else {
                lock.range().len() as off_t
            };
            self.l_pid = lock.owner();
        }
    }
}

/// Create the file range through C flock and opened file reference
fn from_c_flock_and_file(lock: &c_flock, file: Arc<dyn FileLike>) -> Result<FileRange> {
    let start = {
        let whence = RangeLockWhence::try_from(lock.l_whence)?;
        match whence {
            RangeLockWhence::SEEK_SET => lock.l_start,
            RangeLockWhence::SEEK_CUR => (file
                .downcast_ref::<InodeHandle>()
                .ok_or(Error::with_message(Errno::EBADF, "not inode"))?
                .offset() as off_t)
                .checked_add(lock.l_start)
                .ok_or(Error::with_message(Errno::EOVERFLOW, "start overflow"))?,

            RangeLockWhence::SEEK_END => (file.metadata().size as off_t)
                .checked_add(lock.l_start)
                .ok_or(Error::with_message(Errno::EOVERFLOW, "start overflow"))?,
        }
    };

    let (start, end) = match lock.l_len {
        len if len > 0 => {
            let end = start
                .checked_add(len)
                .ok_or(Error::with_message(Errno::EOVERFLOW, "end overflow"))?;
            (start as usize, end as usize)
        }
        0 => (start as usize, OFFSET_MAX),
        len if len < 0 => {
            let end = start;
            let new_start = start + len;
            if new_start < 0 {
                return Err(Error::with_message(Errno::EINVAL, "invalid len"));
            }
            (new_start as usize, end as usize)
        }
        _ => unreachable!(),
    };

    FileRange::new(start, end)
}
