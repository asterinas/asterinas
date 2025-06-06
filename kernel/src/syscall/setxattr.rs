// SPDX-License-Identifier: MPL-2.0

use alloc::borrow::Cow;

use super::SyscallReturn;
use crate::{
    fs::{
        file_handle::FileLike,
        file_table::{get_file_fast, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        path::Dentry,
        utils::{
            XattrName, XattrNamespace, XattrSetFlags, XATTR_NAME_MAX_LEN, XATTR_VALUE_MAX_LEN,
        },
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_setxattr(
    path_ptr: Vaddr,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    setxattr(
        XattrFileCtx::Path(path),
        name_ptr,
        value_ptr,
        value_len,
        flags,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_lsetxattr(
    path_ptr: Vaddr,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    setxattr(
        XattrFileCtx::PathNoFollow(path),
        name_ptr,
        value_ptr,
        value_len,
        flags,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_fsetxattr(
    fd: FileDesc,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let user_space = ctx.user_space();
    setxattr(
        XattrFileCtx::FileHandle(file),
        name_ptr,
        value_ptr,
        value_len,
        flags,
        &user_space,
        ctx,
    )?;

    Ok(SyscallReturn::Return(0))
}

fn setxattr(
    file_ctx: XattrFileCtx,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    flags: i32,
    user_space: &CurrentUserSpace,
    ctx: &Context,
) -> Result<()> {
    let flags = XattrSetFlags::from_bits(flags as _)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid xattr flags"))?;

    let name_cstr = read_xattr_name_cstr_from_user(name_ptr, user_space)?;
    let name_str = name_cstr.to_string_lossy();
    let xattr_name = parse_xattr_name(name_str.as_ref())?;
    check_xattr_namespace(xattr_name.namespace(), ctx)?;

    if value_len > XATTR_VALUE_MAX_LEN {
        return_errno_with_message!(Errno::E2BIG, "xattr value too long");
    }
    let mut value_reader = user_space.reader(value_ptr, value_len)?;

    let dentry = lookup_dentry_for_xattr(&file_ctx, ctx)?;
    dentry.set_xattr(xattr_name, &mut value_reader, flags)
}

/// The context to describe the target file for xattr operations.
pub(super) enum XattrFileCtx<'a> {
    Path(CString),
    PathNoFollow(CString),
    FileHandle(Cow<'a, Arc<dyn FileLike>>),
}

pub(super) fn lookup_dentry_for_xattr<'a>(
    file_ctx: &'a XattrFileCtx<'a>,
    ctx: &'a Context,
) -> Result<Cow<'a, Dentry>> {
    let lookup_dentry_from_fs =
        |path: &CString, ctx: &Context, symlink_no_follow: bool| -> Result<Cow<'_, Dentry>> {
            let path = path.to_string_lossy();
            let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
            let fs = ctx.posix_thread.fs().resolver().read();
            let dentry = if symlink_no_follow {
                fs.lookup_no_follow(&fs_path)?
            } else {
                fs.lookup(&fs_path)?
            };
            Ok(Cow::Owned(dentry))
        };

    match file_ctx {
        XattrFileCtx::Path(path) => lookup_dentry_from_fs(path, ctx, false),
        XattrFileCtx::PathNoFollow(path) => lookup_dentry_from_fs(path, ctx, true),
        XattrFileCtx::FileHandle(file) => {
            let dentry = file.as_inode_or_err()?.dentry();
            Ok(Cow::Borrowed(dentry))
        }
    }
}

pub(super) fn read_xattr_name_cstr_from_user(
    name_ptr: Vaddr,
    user_space: &CurrentUserSpace,
) -> Result<CString> {
    let mut reader = user_space.reader(name_ptr, XATTR_NAME_MAX_LEN + 1)?;
    reader.read_cstring().map_err(|e| {
        if reader.remain() == 0 {
            Error::with_message(Errno::ERANGE, "xattr name too long")
        } else {
            e
        }
    })
}

pub(super) fn parse_xattr_name(name_str: &str) -> Result<XattrName> {
    if name_str.is_empty() || name_str.len() > XATTR_NAME_MAX_LEN {
        return_errno_with_message!(Errno::ERANGE, "xattr name empty or too long");
    }

    let xattr_name = XattrName::try_from_full_name(name_str).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    Ok(xattr_name)
}

pub(super) fn check_xattr_namespace(namespace: XattrNamespace, ctx: &Context) -> Result<()> {
    let credentials = ctx.posix_thread.credentials();
    let permitted_capset = credentials.permitted_capset();
    let effective_capset = credentials.effective_capset();

    if namespace == XattrNamespace::Trusted
        && (!permitted_capset.contains(CapSet::SYS_ADMIN)
            || !effective_capset.contains(CapSet::SYS_ADMIN))
    {
        return_errno_with_message!(
            Errno::EPERM,
            "try to access trusted xattr without CAP_SYS_ADMIN"
        );
    }
    Ok(())
}
