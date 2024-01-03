// SPDX-License-Identifier: MPL-2.0

use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
    utils::{InodeMode, InodeType},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::SYS_SYMLINKAT;

pub fn sys_symlinkat(
    target_addr: Vaddr,
    dirfd: FileDescripter,
    linkpath_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SYMLINKAT);
    let target = read_cstring_from_user(target_addr, MAX_FILENAME_LEN)?;
    let linkpath = read_cstring_from_user(linkpath_addr, MAX_FILENAME_LEN)?;
    debug!(
        "target = {:?}, dirfd = {}, linkpath = {:?}",
        target, dirfd, linkpath
    );

    let current = current!();
    let target = target.to_string_lossy();
    if target.is_empty() {
        return_errno_with_message!(Errno::ENOENT, "target is empty");
    }
    let (dir_dentry, link_name) = {
        let linkpath = linkpath.to_string_lossy();
        if linkpath.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "linkpath is empty");
        }
        if linkpath.ends_with('/') {
            return_errno_with_message!(Errno::EISDIR, "linkpath is dir");
        }
        let fs_path = FsPath::new(dirfd, linkpath.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };

    let new_dentry = dir_dentry.create(
        &link_name,
        InodeType::SymLink,
        InodeMode::from_bits_truncate(0o777),
    )?;
    new_dentry.inode().write_link(&target)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_symlink(target_addr: Vaddr, linkpath_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_symlinkat(target_addr, AT_FDCWD, linkpath_addr)
}
