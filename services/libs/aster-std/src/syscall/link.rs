use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::SYS_LINKAT;

pub fn sys_linkat(
    old_dirfd: FileDescripter,
    old_pathname_addr: Vaddr,
    new_dirfd: FileDescripter,
    new_pathname_addr: Vaddr,
    flags: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_LINKAT);
    let old_pathname = read_cstring_from_user(old_pathname_addr, MAX_FILENAME_LEN)?;
    let new_pathname = read_cstring_from_user(new_pathname_addr, MAX_FILENAME_LEN)?;
    let flags =
        LinkFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "old_dirfd = {}, old_pathname = {:?}, new_dirfd = {}, new_pathname = {:?}, flags = {:?}",
        old_dirfd, old_pathname, new_dirfd, new_pathname, flags
    );

    let current = current!();
    let (old_dentry, new_dir_dentry, new_name) = {
        let old_pathname = old_pathname.to_string_lossy();
        if old_pathname.ends_with('/') {
            return_errno_with_message!(Errno::EPERM, "oldpath is dir");
        }
        if old_pathname.is_empty() && !flags.contains(LinkFlags::AT_EMPTY_PATH) {
            return_errno_with_message!(Errno::ENOENT, "oldpath is empty");
        }
        let new_pathname = new_pathname.to_string_lossy();
        if new_pathname.ends_with('/') || new_pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "newpath is dir or is empty");
        }

        let old_fs_path = FsPath::new(old_dirfd, old_pathname.as_ref())?;
        let new_fs_path = FsPath::new(new_dirfd, new_pathname.as_ref())?;
        let fs = current.fs().read();
        let old_dentry = if flags.contains(LinkFlags::AT_SYMLINK_FOLLOW) {
            fs.lookup(&old_fs_path)?
        } else {
            fs.lookup_no_follow(&old_fs_path)?
        };
        let (new_dir_dentry, new_name) = fs.lookup_dir_and_base_name(&new_fs_path)?;
        (old_dentry, new_dir_dentry, new_name)
    };
    new_dir_dentry.link(&old_dentry, &new_name)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_link(old_pathname_addr: Vaddr, new_pathname_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_linkat(AT_FDCWD, old_pathname_addr, AT_FDCWD, new_pathname_addr, 0)
}

bitflags::bitflags! {
    pub struct LinkFlags: u32 {
        const AT_EMPTY_PATH = 0x1000;
        const AT_SYMLINK_FOLLOW = 0x400;
    }
}
