// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_RMDIR};
use crate::{
    fs::{
        file_table::FileDescripter,
        fs_resolver::{FsPath, AT_FDCWD},
    },
    log_syscall_entry,
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

pub fn sys_rmdir(pathname_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_rmdirat(AT_FDCWD, pathname_addr)
}

pub(super) fn sys_rmdirat(dirfd: FileDescripter, pathname_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RMDIR);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, pathname = {:?}", dirfd, pathname);

    let current = current!();
    let (dir_dentry, name) = {
        let pathname = pathname.to_string_lossy();
        if pathname == "/" {
            return_errno_with_message!(Errno::EBUSY, "is root directory");
        }
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };
    dir_dentry.rmdir(name.trim_end_matches('/'))?;
    Ok(SyscallReturn::Return(0))
}
