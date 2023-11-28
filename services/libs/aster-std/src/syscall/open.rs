use crate::fs::{
    file_handle::FileLike,
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::SYS_OPENAT;

pub fn sys_openat(
    dirfd: FileDescripter,
    pathname_addr: Vaddr,
    flags: u32,
    mode: u16,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_OPENAT);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, pathname = {:?}, flags = {}, mode = {}",
        dirfd, pathname, flags, mode
    );

    let current = current!();
    let file_handle = {
        let pathname = pathname.to_string_lossy();
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        let mask_mode = mode & !current.umask().read().get();
        let inode_handle = current.fs().read().open(&fs_path, flags, mask_mode)?;
        Arc::new(inode_handle)
    };
    let mut file_table = current.file_table().lock();
    let fd = file_table.insert(file_handle);
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_open(pathname_addr: Vaddr, flags: u32, mode: u16) -> Result<SyscallReturn> {
    self::sys_openat(AT_FDCWD, pathname_addr, flags, mode)
}

/// File for output busybox ash log.
struct BusyBoxTraceFile;

impl FileLike for BusyBoxTraceFile {
    fn write(&self, buf: &[u8]) -> Result<usize> {
        debug!("ASH TRACE: {}", core::str::from_utf8(buf)?);
        Ok(buf.len())
    }
}
