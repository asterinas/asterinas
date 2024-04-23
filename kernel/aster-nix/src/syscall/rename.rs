// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_RENAMEAT};
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::InodeType,
    },
    log_syscall_entry,
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

pub fn sys_renameat(
    old_dirfd: FileDesc,
    old_path_addr: Vaddr,
    new_dirfd: FileDesc,
    new_path_addr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RENAMEAT);
    let old_path = read_cstring_from_user(old_path_addr, MAX_FILENAME_LEN)?;
    let new_path = read_cstring_from_user(new_path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "old_dirfd = {}, old_path = {:?}, new_dirfd = {}, new_path = {:?}",
        old_dirfd, old_path, new_dirfd, new_path
    );

    let current = current!();
    let fs = current.fs().read();

    let (old_dir_dentrymnt, old_name) = {
        let old_path = old_path.to_string_lossy();
        if old_path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "oldpath is empty");
        }
        let old_fs_path = FsPath::new(old_dirfd, old_path.as_ref())?;
        fs.lookup_dir_and_base_name(&old_fs_path)?
    };
    let old_dentrymnt = old_dir_dentrymnt.lookup(&old_name)?;

    let (new_dir_dentrymnt, new_name) = {
        let new_path = new_path.to_string_lossy();
        if new_path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "newpath is empty");
        }
        if new_path.ends_with('/') && old_dentrymnt.type_() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "oldpath is not dir");
        }
        let new_fs_path = FsPath::new(new_dirfd, new_path.as_ref().trim_end_matches('/'))?;
        fs.lookup_dir_and_base_name(&new_fs_path)?
    };

    // Check abs_path
    let old_abs_path = old_dentrymnt.abs_path();
    let new_abs_path = new_dir_dentrymnt.abs_path() + "/" + &new_name;
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

    old_dir_dentrymnt.rename(&old_name, &new_dir_dentrymnt, &new_name)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_rename(old_path_addr: Vaddr, new_path_addr: Vaddr) -> Result<SyscallReturn> {
    self::sys_renameat(AT_FDCWD, old_path_addr, AT_FDCWD, new_path_addr)
}
