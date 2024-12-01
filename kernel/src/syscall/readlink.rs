// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_readlinkat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    usr_buf_addr: Vaddr,
    usr_buf_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, path = {:?}, usr_buf_addr = 0x{:x}, usr_buf_len = 0x{:x}",
        dirfd, path, usr_buf_addr, usr_buf_len
    );

    let dentry = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        ctx.posix_thread
            .fs()
            .resolver()
            .read()
            .lookup_no_follow(&fs_path)?
    };
    let linkpath = dentry.inode().read_link()?;
    let bytes = linkpath.as_bytes();
    let write_len = bytes.len().min(usr_buf_len);
    user_space.write_bytes(usr_buf_addr, &mut VmReader::from(&bytes[..write_len]))?;
    Ok(SyscallReturn::Return(write_len as _))
}

pub fn sys_readlink(
    path_addr: Vaddr,
    usr_buf_addr: Vaddr,
    usr_buf_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    self::sys_readlinkat(AT_FDCWD, path_addr, usr_buf_addr, usr_buf_len, ctx)
}
