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
use super::SYS_MKDIRAT;

pub fn sys_mkdirat(
    dirfd: FileDescripter,
    pathname_addr: Vaddr,
    mode: u16,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MKDIRAT);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, pathname = {:?}, mode = {}",
        dirfd, pathname, mode
    );

    let current = current!();
    let (dir_dentry, name) = {
        let pathname = pathname.to_string_lossy();
        if pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };

    let inode_mode = {
        let mask_mode = mode & !current.umask().read().get();
        InodeMode::from_bits_truncate(mask_mode)
    };
    let _ = dir_dentry.create(name.trim_end_matches('/'), InodeType::Dir, inode_mode)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_mkdir(pathname_addr: Vaddr, mode: u16) -> Result<SyscallReturn> {
    self::sys_mkdirat(AT_FDCWD, pathname_addr, mode)
}
