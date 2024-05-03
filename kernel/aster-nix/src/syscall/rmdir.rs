// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

pub fn sys_rmdir(path_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_rmdirat(AT_FDCWD, path_addr)
}

pub(super) fn sys_rmdirat(dirfd: FileDesc, path_addr: Vaddr) -> Result<SyscallReturn> {
    let path_addr = read_cstring_from_user(path_addr, MAX_FILENAME_LEN)?;
    debug!("dirfd = {}, path_addr = {:?}", dirfd, path_addr);

    let current = current!();
    let (dir_dentry, name) = {
        let path_addr = path_addr.to_string_lossy();
        if path_addr == "/" {
            return_errno_with_message!(Errno::EBUSY, "is root directory");
        }
        let fs_path = FsPath::new(dirfd, path_addr.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };
    dir_dentry.rmdir(name.trim_end_matches('/'))?;
    Ok(SyscallReturn::Return(0))
}
