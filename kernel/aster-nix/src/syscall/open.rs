// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_handle::FileLike,
        file_table::{FdFlags, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        utils::CreationFlags,
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
    // CreationFlags::O_CREAT | AccessMode::O_WRONLY | CreationFlags::O_TRUNC;
    let flags: u32 = 0x241;
    self::sys_openat(AT_FDCWD, path_addr, flags, mode)
}

/// File for output busybox ash log.
struct BusyBoxTraceFile;

impl FileLike for BusyBoxTraceFile {
    fn write(&self, buf: &[u8]) -> Result<usize> {
        debug!("ASH TRACE: {}", core::str::from_utf8(buf)?);
        Ok(buf.len())
    }
}
