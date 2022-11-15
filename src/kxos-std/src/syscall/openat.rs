use crate::fs::file::FileDescripter;
use crate::memory::read_cstring_from_user;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;

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
    if dirfd == AT_FDCWD && pathname == CString::new("/etc/passwd")? {
        return_errno!(Errno::ENOENT);
    }
    todo!()
}
