// SPDX-License-Identifier: MPL-2.0

use super::{
    SyscallReturn,
    setxattr::{XattrFileCtx, current_can_access_trusted_xattr, lookup_path_for_xattr},
};
use crate::{
    fs::{
        file::file_table::{RawFileDesc, get_file_fast},
        vfs::xattr::{XATTR_LIST_MAX_LEN, XattrNamespace},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_listxattr(
    path_ptr: Vaddr,
    list_ptr: Vaddr, // The given list is used to place xattr (null-terminated) names.
    list_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    let len = listxattr(
        XattrFileCtx::Path(path),
        list_ptr,
        list_len,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(len as _))
}

pub fn sys_llistxattr(
    path_ptr: Vaddr,
    list_ptr: Vaddr, // The given list is used to place xattr (null-terminated) names.
    list_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    let len = listxattr(
        XattrFileCtx::PathNoFollow(path),
        list_ptr,
        list_len,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(len as _))
}

pub fn sys_flistxattr(
    raw_fd: RawFileDesc,
    list_ptr: Vaddr, // The given list is used to place xattr (null-terminated) names.
    list_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);

    let user_space = ctx.user_space();
    let len = listxattr(
        XattrFileCtx::FileHandle(file),
        list_ptr,
        list_len,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(len as _))
}

fn listxattr(
    file_ctx: XattrFileCtx,
    list_ptr: Vaddr,
    list_len: usize,
    user_space: &CurrentUserSpace,
    ctx: &Context,
) -> Result<usize> {
    if list_len > XATTR_LIST_MAX_LEN {
        return_errno_with_message!(Errno::E2BIG, "xattr list too long");
    }

    let namespace = get_current_xattr_namespace(ctx);
    let mut list_writer = user_space.writer(list_ptr, list_len)?;

    let path = lookup_path_for_xattr(&file_ctx, ctx)?;
    path.list_xattr(namespace, &mut list_writer)
}

fn get_current_xattr_namespace(ctx: &Context) -> XattrNamespace {
    if current_can_access_trusted_xattr(ctx) {
        XattrNamespace::Trusted
    } else {
        XattrNamespace::User
    }
}
