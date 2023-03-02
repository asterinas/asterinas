use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
    utils::Stat,
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;
use crate::util::write_val_to_user;

use super::SyscallReturn;
use super::{SYS_FSTAT, SYS_FSTATAT};

pub fn sys_fstat(fd: FileDescripter, stat_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FSTAT);
    debug!("fd = {}, stat_buf_addr = 0x{:x}", fd, stat_buf_ptr);

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    let stat = Stat::from(file.metadata());
    write_val_to_user(stat_buf_ptr, &stat)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_stat(filename_ptr: Vaddr, stat_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    self::sys_fstatat(AT_FDCWD, filename_ptr, stat_buf_ptr, 0)
}

pub fn sys_lstat(filename_ptr: Vaddr, stat_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    self::sys_fstatat(
        AT_FDCWD,
        filename_ptr,
        stat_buf_ptr,
        StatFlags::AT_SYMLINK_NOFOLLOW.bits(),
    )
}

pub fn sys_fstatat(
    dirfd: FileDescripter,
    filename_ptr: Vaddr,
    stat_buf_ptr: Vaddr,
    flags: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FSTATAT);
    let filename = read_cstring_from_user(filename_ptr, MAX_FILENAME_LEN)?;
    let flags =
        StatFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "dirfd = {}, filename = {:?}, stat_buf_ptr = 0x{:x}, flags = {:?}",
        dirfd, filename, stat_buf_ptr, flags
    );
    let current = current!();
    let dentry = {
        let filename = filename.to_string_lossy();
        if filename.is_empty() && !flags.contains(StatFlags::AT_EMPTY_PATH) {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, filename.as_ref())?;
        let fs = current.fs().read();
        if flags.contains(StatFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_no_follow(&fs_path)?
        } else {
            fs.lookup(&fs_path)?
        }
    };
    let stat = Stat::from(dentry.inode_metadata());
    write_val_to_user(stat_buf_ptr, &stat)?;
    Ok(SyscallReturn::Return(0))
}

bitflags::bitflags! {
    struct StatFlags: u32 {
        const AT_EMPTY_PATH = 1 << 12;
        const AT_NO_AUTOMOUNT = 1 << 11;
        const AT_SYMLINK_NOFOLLOW = 1 << 8;
    }
}
