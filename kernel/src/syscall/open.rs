// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        notify::fsnotify_open,
        utils::{AccessMode, CreationFlags},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_openat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    flags: u32,
    mode: u16,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, path = {:?}, flags = {}, mode = {}",
        dirfd, path, flags, mode
    );

    if path.is_empty() {
        return_errno_with_message!(Errno::ENOENT, "openat fails with empty path");
    }

    let file_handle = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        let fs_ref = ctx.thread_local.borrow_fs();
        let mask_mode = mode & !fs_ref.umask().get();
        let inode_handle = fs_ref
            .resolver()
            .read()
            .open(&fs_path, flags, mask_mode)
            .map_err(|err| match err.error() {
                Errno::EINTR => Error::new(Errno::ERESTARTSYS),
                _ => err,
            })?;
        Arc::new(inode_handle)
    };

    let fd = {
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        let fd_flags =
            if CreationFlags::from_bits_truncate(flags).contains(CreationFlags::O_CLOEXEC) {
                FdFlags::CLOEXEC
            } else {
                FdFlags::empty()
            };
        file_table_locked.insert(file_handle.clone(), fd_flags)
    };
    fsnotify_open(file_handle.path())?;
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_open(path_addr: Vaddr, flags: u32, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_openat(AT_FDCWD, path_addr, flags, mode, ctx)
}

pub fn sys_creat(path_addr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    let flags =
        AccessMode::O_WRONLY as u32 | CreationFlags::O_CREAT.bits() | CreationFlags::O_TRUNC.bits();
    self::sys_openat(AT_FDCWD, path_addr, flags, mode, ctx)
}
