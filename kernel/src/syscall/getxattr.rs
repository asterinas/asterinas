// SPDX-License-Identifier: MPL-2.0

use super::{
    setxattr::{
        check_xattr_namespace, lookup_path_for_xattr, parse_xattr_name,
        read_xattr_name_cstr_from_user, XattrFileCtx,
    },
    SyscallReturn,
};
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        utils::XATTR_VALUE_MAX_LEN,
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_getxattr(
    path_ptr: Vaddr,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    let len = getxattr(
        XattrFileCtx::Path(path),
        name_ptr,
        value_ptr,
        value_len,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(len as _))
}

pub fn sys_lgetxattr(
    path_ptr: Vaddr,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    let len = getxattr(
        XattrFileCtx::PathNoFollow(path),
        name_ptr,
        value_ptr,
        value_len,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(len as _))
}

pub fn sys_fgetxattr(
    fd: FileDesc,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let user_space = ctx.user_space();
    let len = getxattr(
        XattrFileCtx::FileHandle(file),
        name_ptr,
        value_ptr,
        value_len,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(len as _))
}

fn getxattr(
    file_ctx: XattrFileCtx,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    user_space: &CurrentUserSpace,
    ctx: &Context,
) -> Result<usize> {
    let name_cstr = read_xattr_name_cstr_from_user(name_ptr, user_space)?;
    let name_str = name_cstr.to_string_lossy();
    let xattr_name = parse_xattr_name(name_str.as_ref())?;
    check_xattr_namespace(xattr_name.namespace(), ctx).map_err(|_| Error::new(Errno::ENODATA))?;

    let mut value_writer = user_space.writer(value_ptr, value_len.min(XATTR_VALUE_MAX_LEN))?;

    let path = lookup_path_for_xattr(&file_ctx, ctx)?;
    path.get_xattr(xattr_name, &mut value_writer)
}
