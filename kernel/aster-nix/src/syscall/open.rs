// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        utils::{AccessMode, CreationFlags},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    util::read_cstring_from_user,
};

pub fn sys_openat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    flags: u32,
    mode: u16,
) -> Result<SyscallReturn> {
    let path = read_cstring_from_user(path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, path = {:?}, flags = {}, mode = {}",
        dirfd, path, flags, mode
    );

    let current = current!();
    let file_handle = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        let mask_mode = mode & !current.umask().read().get();
        let inode_handle = current.fs().read().open(&fs_path, flags, mask_mode)?;
        Arc::new(inode_handle)
    };
    let mut file_table = current.file_table().lock();
    let fd = {
        let fd_flags =
            if CreationFlags::from_bits_truncate(flags).contains(CreationFlags::O_CLOEXEC) {
                FdFlags::CLOEXEC
            } else {
                FdFlags::empty()
            };
        file_table.insert(file_handle, fd_flags)
    };
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_open(path_addr: Vaddr, flags: u32, mode: u16) -> Result<SyscallReturn> {
    self::sys_openat(AT_FDCWD, path_addr, flags, mode)
}

pub fn sys_creat(path_addr: Vaddr, mode: u16) -> Result<SyscallReturn> {
    let flags =
        AccessMode::O_WRONLY as u32 | CreationFlags::O_CREAT.bits() | CreationFlags::O_TRUNC.bits();
    self::sys_openat(AT_FDCWD, path_addr, flags, mode)
}
