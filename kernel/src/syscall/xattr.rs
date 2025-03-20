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
            XattrFlags, XattrNamespace, XATTR_LIST_MAX_LEN, XATTR_NAME_MAX_LEN, XATTR_VALUE_MAX_LEN,
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
    let mut file_table = ctx.thread_local.file_table().borrow_mut();
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
    let flags = XattrFlags::from_bits(flags as _)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid xattr flags"))?;

    let name = read_xattr_name_from_user(name_ptr, user_space)?;
    let (name, namespace) = parse_xattr_name(&name)?;
    check_namespace(namespace, ctx)?;

    if value_len > XATTR_VALUE_MAX_LEN {
        return_errno_with_message!(Errno::E2BIG, "xattr value too long");
    }
    let mut value_reader = user_space.reader(value_ptr, value_len)?;

    let dentry = target_dentry(&file_ctx, ctx)?;
    dentry.set_xattr(namespace, name.as_ref(), &mut value_reader, flags)
}

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
    let mut file_table = ctx.thread_local.file_table().borrow_mut();
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
    let name = read_xattr_name_from_user(name_ptr, user_space)?;
    let (name, namespace) = parse_xattr_name(&name)?;
    check_namespace(namespace, ctx).map_err(|_| Error::new(Errno::ENODATA))?;

    let mut value_writer = user_space.writer(value_ptr, value_len.min(XATTR_VALUE_MAX_LEN))?;

    let dentry = target_dentry(&file_ctx, ctx)?;
    dentry.get_xattr(namespace, name.as_ref(), &mut value_writer)
}

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
    fd: FileDesc,
    list_ptr: Vaddr, // The given list is used to place xattr (null-terminated) names.
    list_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.file_table().borrow_mut();
    let file = get_file_fast!(&mut file_table, fd);

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

    let namespace = get_current_namespace(ctx);
    let mut list_writer = user_space.writer(list_ptr, list_len)?;

    let dentry = target_dentry(&file_ctx, ctx)?;
    dentry.list_xattr(namespace, &mut list_writer)
}

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
    let mut file_table = ctx.thread_local.file_table().borrow_mut();
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
    let name = read_xattr_name_from_user(name_ptr, user_space)?;
    let (name, namespace) = parse_xattr_name(&name)?;
    check_namespace(namespace, ctx)?;

    let dentry = target_dentry(&file_ctx, ctx)?;
    dentry.remove_xattr(namespace, name.as_ref())
}

/// The context to describe the target file for xattr operations.
enum XattrFileCtx<'a> {
    Path(CString),
    PathNoFollow(CString),
    FileHandle(Cow<'a, Arc<dyn FileLike>>),
}

fn target_dentry<'a>(file_ctx: &'a XattrFileCtx<'a>, ctx: &'a Context) -> Result<Cow<'a, Dentry>> {
    match file_ctx {
        XattrFileCtx::Path(path) => lookup_dentry(path, ctx, false),
        XattrFileCtx::PathNoFollow(path) => lookup_dentry(path, ctx, true),
        XattrFileCtx::FileHandle(file) => {
            let dentry = file.as_inode_or_err()?.dentry();
            Ok(Cow::Borrowed(dentry))
        }
    }
}

fn lookup_dentry<'a>(
    path: &'a CString,
    ctx: &'a Context,
    symlink_no_follow: bool,
) -> Result<Cow<'a, Dentry>> {
    let path = path.to_string_lossy();
    let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
    let fs = ctx.posix_thread.fs().resolver().read();
    let dentry = if symlink_no_follow {
        fs.lookup_no_follow(&fs_path)?
    } else {
        fs.lookup(&fs_path)?
    };
    Ok(Cow::Owned(dentry))
}

fn read_xattr_name_from_user<'a>(
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

fn parse_xattr_name<'a>(name_cstr: &'a CString) -> Result<(Cow<'a, str>, XattrNamespace)> {
    let name = name_cstr.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return_errno_with_message!(Errno::ERANGE, "xattr name empty or too long");
    }

    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;

    Ok((name, namespace))
}

fn check_namespace(namespace: XattrNamespace, ctx: &Context) -> Result<()> {
    let credentials = ctx.posix_thread.credentials();
    let permitted_capset = credentials.permitted_capset();
    let effective_capset = credentials.effective_capset();

    match namespace {
        XattrNamespace::Trusted => {
            if !permitted_capset.contains(CapSet::SYS_ADMIN)
                || !effective_capset.contains(CapSet::SYS_ADMIN)
            {
                return_errno_with_message!(
                    Errno::EPERM,
                    "try to access trusted xattr without CAP_SYS_ADMIN"
                );
            }
        }
        _ => {}
    }
    Ok(())
}

fn get_current_namespace(ctx: &Context) -> XattrNamespace {
    let credentials = ctx.posix_thread.credentials();
    let permitted_capset = credentials.permitted_capset();
    let effective_capset = credentials.effective_capset();

    if permitted_capset.contains(CapSet::SYS_ADMIN) && effective_capset.contains(CapSet::SYS_ADMIN)
    {
        XattrNamespace::Trusted
    } else {
        XattrNamespace::User
    }
}
