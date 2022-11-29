use crate::fs::file::File;
use crate::fs::file::FileDescripter;
use crate::memory::read_cstring_from_user;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::tty::get_console;

use super::SyscallReturn;
use super::SYS_OPENAT;

const AT_FDCWD: FileDescripter = -100;

pub fn sys_openat(
    dirfd: FileDescripter,
    pathname_addr: Vaddr,
    flags: i32,
    mode: u16,
) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_OPENAT]", SYS_OPENAT);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, pathname = {:?}, flags = {}, mode = {}",
        dirfd, pathname, flags, mode
    );

    // TODO: do real openat

    // Below are three special files we encountered when running busybox ash.
    // We currently only return ENOENT, which means the file does not exist.
    if dirfd == AT_FDCWD && pathname == CString::new("/etc/passwd")? {
        return_errno!(Errno::ENOENT);
    }

    if dirfd == AT_FDCWD && pathname == CString::new("/etc/profile")? {
        return_errno!(Errno::ENOENT);
    }

    if dirfd == AT_FDCWD && pathname == CString::new("./trace")? {
        return_errno!(Errno::ENOENT);
    }

    if dirfd == AT_FDCWD && pathname == CString::new("/dev/tty")? {
        let tty_file = get_console().clone() as Arc<dyn File>;
        let current = current!();
        let mut file_table = current.file_table().lock();
        let fd = file_table.insert(tty_file);
        debug!("openat fd = {}", fd);
        return Ok(SyscallReturn::Return(fd as _));
    }
    todo!()
}
