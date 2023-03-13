use crate::fs::{
    file_handle::{File, FileHandle},
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::tty::get_n_tty;
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
        "dirfd = {}, pathname = {:?}, flags = {}, mode = 0o{:o}",
        dirfd, pathname, flags, mode
    );

    // TODO: do real openat

    // Below are three special files we encountered when running busybox ash.
    // We currently only return ENOENT, which means the file does not exist.
    if dirfd == AT_FDCWD && pathname == CString::new("/etc/passwd")? {
        return_errno_with_message!(Errno::ENOENT, "No such file");
    }

    if dirfd == AT_FDCWD && pathname == CString::new("/etc/profile")? {
        return_errno_with_message!(Errno::ENOENT, "No such file");
    }

    if dirfd == AT_FDCWD && pathname == CString::new("./trace")? {
        // Debug use: This file is used for output busybox log
        let trace_file = FileHandle::new_file(Arc::new(BusyBoxTraceFile) as Arc<dyn File>);
        let current = current!();
        let mut file_table = current.file_table().lock();
        let fd = file_table.insert(trace_file);
        return Ok(SyscallReturn::Return(fd as _));
    }

    if dirfd == AT_FDCWD && pathname == CString::new("/dev/tty")? {
        let tty_file = FileHandle::new_file(get_n_tty().clone() as Arc<dyn File>);
        let current = current!();
        let mut file_table = current.file_table().lock();
        let fd = file_table.insert(tty_file);
        return Ok(SyscallReturn::Return(fd as _));
    }

    // The common path
    let current = current!();
    let file_handle = {
        let pathname = pathname.to_string_lossy();
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        let inode_handle = current.fs().read().open(&fs_path, flags, mode)?;
        FileHandle::new_inode_handle(inode_handle)
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

impl File for BusyBoxTraceFile {
    fn write(&self, buf: &[u8]) -> Result<usize> {
        debug!("ASH TRACE: {}", core::str::from_utf8(buf)?);
        Ok(buf.len())
    }
}
