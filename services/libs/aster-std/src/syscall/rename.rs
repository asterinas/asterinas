use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
    utils::InodeType,
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::SYS_RENAMEAT;

pub fn sys_renameat(
    old_dirfd: FileDescripter,
    old_pathname_addr: Vaddr,
    new_dirfd: FileDescripter,
    new_pathname_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RENAMEAT);
    let old_pathname = read_cstring_from_user(old_pathname_addr, MAX_FILENAME_LEN)?;
    let new_pathname = read_cstring_from_user(new_pathname_addr, MAX_FILENAME_LEN)?;
    debug!(
        "old_dirfd = {}, old_pathname = {:?}, new_dirfd = {}, new_pathname = {:?}",
        old_dirfd, old_pathname, new_dirfd, new_pathname
    );

    let current = current!();
    let fs = current.fs().read();

    let (old_dir_dentry, old_name) = {
        let old_pathname = old_pathname.to_string_lossy();
        if old_pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "oldpath is empty");
        }
        let old_fs_path = FsPath::new(old_dirfd, old_pathname.as_ref())?;
        fs.lookup_dir_and_base_name(&old_fs_path)?
    };
    let old_dentry = old_dir_dentry.lookup(&old_name)?;

    let (new_dir_dentry, new_name) = {
        let new_pathname = new_pathname.to_string_lossy();
        if new_pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "newpath is empty");
        }
        if new_pathname.ends_with('/') && old_dentry.inode_type() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "oldpath is not dir");
        }
        let new_fs_path = FsPath::new(new_dirfd, new_pathname.as_ref().trim_end_matches('/'))?;
        fs.lookup_dir_and_base_name(&new_fs_path)?
    };

    // Check abs_path
    let old_abs_path = old_dentry.abs_path();
    let new_abs_path = new_dir_dentry.abs_path() + "/" + &new_name;
    if new_abs_path.starts_with(&old_abs_path) {
        if new_abs_path.len() == old_abs_path.len() {
            return Ok(SyscallReturn::Return(0));
        } else {
            return_errno_with_message!(
                Errno::EINVAL,
                "newpath contains a path prefix of the oldpath"
            );
        }
    }

    old_dir_dentry.rename(&old_name, &new_dir_dentry, &new_name)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_rename(old_pathname_addr: Vaddr, new_pathname_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_renameat(AT_FDCWD, old_pathname_addr, AT_FDCWD, new_pathname_addr)
}
