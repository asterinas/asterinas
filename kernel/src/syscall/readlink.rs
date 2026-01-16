// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        path::{AT_FDCWD, FsPath},
        utils::SymbolicLink,
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

    let link_path = {
        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();

        let path_name = path_name.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(dirfd, &path_name)?;
        let path = path_resolver.lookup_no_follow(&fs_path)?;

        match path.inode().read_link()? {
            SymbolicLink::Plain(s) => s,
            SymbolicLink::Path(path) => path_resolver.make_abs_path(&path).into_string(),
        }
    };

    let bytes = link_path.as_bytes();
    let write_len = bytes.len().min(usr_buf_len);
    user_space.write_bytes(usr_buf_addr, &bytes[..write_len])?;
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
