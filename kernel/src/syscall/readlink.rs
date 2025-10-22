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
    let path_name = user_space.read_cstring(path_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, path = {:?}, usr_buf_addr = 0x{:x}, usr_buf_len = 0x{:x}",
        dirfd, path_name, usr_buf_addr, usr_buf_len
    );

    let path = {
        let path_name = path_name.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(dirfd, &path_name)?;
        ctx.thread_local
            .borrow_fs()
            .resolver()
            .read()
            .lookup_no_follow(&fs_path)?
    };

    let linkpath = path.inode().read_link()?.into_string();
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
