use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::{read_cstring_from_user, write_bytes_to_user};

use super::SyscallReturn;
use super::SYS_READLINKAT;

pub fn sys_readlinkat(
    dirfd: FileDescripter,
    pathname_addr: Vaddr,
    usr_buf_addr: Vaddr,
    usr_buf_len: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_READLINKAT);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, pathname = {:?}, usr_buf_addr = 0x{:x}, usr_buf_len = 0x{:x}",
        dirfd, pathname, usr_buf_addr, usr_buf_len
    );

    let current = current!();
    let dentry = {
        let pathname = pathname.to_string_lossy();
        if pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        current.fs().read().lookup_no_follow(&fs_path)?
    };
    let linkpath = dentry.inode().read_link()?;
    let bytes = linkpath.as_bytes();
    let write_len = bytes.len().min(usr_buf_len);
    write_bytes_to_user(usr_buf_addr, &bytes[..write_len])?;
    Ok(SyscallReturn::Return(write_len as _))
}

pub fn sys_readlink(
    pathname_addr: Vaddr,
    usr_buf_addr: Vaddr,
    usr_buf_len: usize,
) -> Result<SyscallReturn> {
    self::sys_readlinkat(AT_FDCWD, pathname_addr, usr_buf_addr, usr_buf_len)
}
