// SPDX-License-Identifier: MPL-2.0

use super::{
    setxattr::{
        check_xattr_namespace, lookup_path_for_xattr, parse_xattr_name,
        read_xattr_name_cstr_from_user, XattrFileCtx,
    },
    SyscallReturn,
};
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_removexattr(path_ptr: Vaddr, name_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    removexattr(XattrFileCtx::Path(path), name_ptr, &user_space, ctx)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_lremovexattr(path_ptr: Vaddr, name_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    removexattr(XattrFileCtx::PathNoFollow(path), name_ptr, &user_space, ctx)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_fremovexattr(fd: FileDesc, name_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let user_space = ctx.user_space();
    removexattr(XattrFileCtx::FileHandle(file), name_ptr, &user_space, ctx)?;

    Ok(SyscallReturn::Return(0))
}

fn removexattr(
    file_ctx: XattrFileCtx,
    name_ptr: Vaddr,
    user_space: &CurrentUserSpace,
    ctx: &Context,
) -> Result<()> {
    let name_cstr = read_xattr_name_cstr_from_user(name_ptr, user_space)?;
    let name_str = name_cstr.to_string_lossy();
    let xattr_name = parse_xattr_name(name_str.as_ref())?;
    check_xattr_namespace(xattr_name.namespace(), ctx)?;

    let path = lookup_path_for_xattr(&file_ctx, ctx)?;
    path.remove_xattr(xattr_name)
}
