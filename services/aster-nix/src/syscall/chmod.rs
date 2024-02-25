// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_FCHMOD, SYS_FCHMODAT};
use crate::{
    fs::{
        file_table::FileDescripter,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::{InodeMode, PATH_MAX},
    },
    log_syscall_entry,
    prelude::*,
    util::read_cstring_from_user,
};

pub fn sys_fchmod(fd: FileDescripter, mode: u16) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHMOD);
    debug!("fd = {}, mode = 0o{:o}", fd, mode);

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    file.set_mode(InodeMode::from_bits_truncate(mode))?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_chmod(path_ptr: Vaddr, mode: u16) -> Result<SyscallReturn> {
    self::sys_fchmodat(AT_FDCWD, path_ptr, mode)
}

// Glibc handles the `flags` argument, so we just ignore it.
pub fn sys_fchmodat(
    dirfd: FileDescripter,
    path_ptr: Vaddr,
    mode: u16,
    /* flags: u32, */
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FCHMODAT);
    let path = read_cstring_from_user(path_ptr, PATH_MAX)?;
    debug!("dirfd = {}, path = {:?}, mode = 0o{:o}", dirfd, path, mode,);

    let current = current!();
    let dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        current.fs().read().lookup(&fs_path)?
    };
    dentry.set_mode(InodeMode::from_bits_truncate(mode))?;
    Ok(SyscallReturn::Return(0))
}
