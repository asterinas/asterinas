// SPDX-License-Identifier: MPL-2.0

use alloc::borrow::Cow;

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file::{
            FileLike, StatusFlags,
            file_table::{RawFileDesc, get_file_fast},
        },
        vfs::{
            inode::Inode,
            path::{AT_FDCWD, EmptyPathStr, FsPath, Path},
            xattr::{
                self, XATTR_NAME_MAX_LEN, XATTR_VALUE_MAX_LEN, XattrName, XattrNamespace,
                XattrSetFlags,
            },
        },
    },
    prelude::*,
    process::{
        UserNamespace,
        credentials::{FileCapabilities, Gid, Uid, VfsCapRevision, capabilities::CapSet},
    },
    security::lsm::hooks as lsm_hooks,
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
    raw_fd: RawFileDesc,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);

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
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid xattr flags"))?;

    let name_cstr = read_xattr_name_cstr_from_user(name_ptr, user_space)?;
    let name_str = name_cstr.to_string_lossy();
    let xattr_name = parse_xattr_name(name_str.as_ref())?;
    check_xattr_namespace(xattr_name.namespace(), ctx)?;

    if value_len > XATTR_VALUE_MAX_LEN {
        return_errno_with_message!(Errno::E2BIG, "xattr value too long");
    }
    let mut value_reader = user_space.reader(value_ptr, value_len)?;
    let path = lookup_path_for_xattr(&file_ctx, ctx)?;
    if xattr_name.full_name() != xattr::SECURITY_CAPABILITY_XATTR_NAME {
        path.set_xattr(xattr_name, &mut value_reader, flags)?;
    } else {
        if value_len == 0 {
            return_errno_with_message!(Errno::EINVAL, "file capability xattr cannot be empty");
        }

        let mut value = [0u8; FileCapabilities::MAX_XATTR_SIZE];
        let value_read_len = value_len.min(FileCapabilities::MAX_XATTR_SIZE);
        value_reader.read_fallible(&mut VmWriter::from(&mut value[..value_read_len]))?;
        let raw_value = &value[..value_read_len];

        let header = FileCapabilities::read_u32_le(raw_value, 0)?;
        let (revision, _) = FileCapabilities::parse_header(header, value_len)?;
        if revision == VfsCapRevision::V1 {
            return_errno_with_message!(
                Errno::EINVAL,
                "V1 file capability xattrs cannot be written"
            );
        }
        if revision == VfsCapRevision::V3
            && Uid::new(FileCapabilities::read_u32_le(raw_value, 5)?) == Uid::INVALID
        {
            return_errno_with_message!(
                Errno::EINVAL,
                "file capability root UID does not map to a valid UID"
            );
        }

        // With only the initial user namespace and no idmapped mounts, valid V2/V3 values use
        // the same representation in the caller and filesystem namespaces.
        //
        // FIXME: Convert V2/V3 root IDs when user namespaces or idmapped mounts are supported.
        // Reference: <https://elixir.bootlin.com/linux/v7.1/source/security/commoncap.c#L550>.
        let inode = path.inode();
        capable_modify_file_cap(inode.as_ref(), ctx)?;

        let mut value_reader = VmReader::from(raw_value).to_fallible();
        // Linux leaves authorization for `security.*` xattrs to the security modules rather than
        // requiring ordinary DAC write permission. The checks above authorize this specific
        // security xattr, so bypass the generic `Path::set_xattr` DAC check.
        inode.set_xattr(xattr_name, &mut value_reader, flags)?;
    }
    fs::vfs::notify::on_attr_change(&path);
    Ok(())
}

/// The context to describe the target file for xattr operations.
pub(super) enum XattrFileCtx<'a> {
    Path(CString),
    PathNoFollow(CString),
    FileHandle(Cow<'a, Arc<dyn FileLike>>),
}

pub(super) fn lookup_path_for_xattr<'a>(
    file_ctx: &'a XattrFileCtx<'a>,
    ctx: &'a Context,
) -> Result<Cow<'a, Path>> {
    let lookup_path_from_fs =
        |path: &CString, ctx: &Context, symlink_no_follow: bool| -> Result<Cow<'_, Path>> {
            let path = path.to_string_lossy();
            let fs_path = FsPath::from_fd_at(AT_FDCWD, &path, EmptyPathStr::Reject)?;
            let fs_ref = ctx.thread_local.borrow_fs();
            let path_resolver = fs_ref.resolver().read();
            let path = if symlink_no_follow {
                path_resolver.lookup_no_follow(&fs_path)?
            } else {
                path_resolver.lookup(&fs_path)?
            };
            Ok(Cow::Owned(path))
        };

    match file_ctx {
        XattrFileCtx::Path(path) => lookup_path_from_fs(path, ctx, false),
        XattrFileCtx::PathNoFollow(path) => lookup_path_from_fs(path, ctx, true),
        XattrFileCtx::FileHandle(file) => {
            let file = file.as_inode_handle_or_err()?;
            if file.status_flags().contains(StatusFlags::O_PATH) {
                return_errno_with_message!(Errno::EBADF, "the file is opened as a path");
            }
            let path = file.path();
            Ok(Cow::Borrowed(path))
        }
    }
}

pub(super) fn read_xattr_name_cstr_from_user(
    name_ptr: Vaddr,
    user_space: &CurrentUserSpace,
) -> Result<CString> {
    user_space
        .read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)
        .map_err(|err| {
            if err.error() == Errno::ENAMETOOLONG {
                Error::with_message(Errno::ERANGE, "xattr name too long")
            } else {
                err
            }
        })
}

pub(super) fn parse_xattr_name(name_str: &str) -> Result<XattrName<'_>> {
    if name_str.is_empty() || name_str.len() > XATTR_NAME_MAX_LEN {
        return_errno_with_message!(Errno::ERANGE, "xattr name empty or too long");
    }

    let xattr_name = XattrName::try_from_full_name(name_str)
        .ok_or_else(|| Error::with_message(Errno::EOPNOTSUPP, "invalid xattr namespace"))?;
    Ok(xattr_name)
}

pub(super) fn check_xattr_namespace(namespace: XattrNamespace, ctx: &Context) -> Result<()> {
    if namespace != XattrNamespace::Trusted {
        return Ok(());
    }

    if !ctx
        .posix_thread
        .credentials()
        .permitted_capset()
        .contains(CapSet::SYS_ADMIN)
    {
        return_errno_with_message!(
            Errno::EPERM,
            "try to access trusted xattr without permitted CAP_SYS_ADMIN"
        );
    }

    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        ctx.posix_thread,
        CapSet::SYS_ADMIN,
    ))
}

pub(super) fn capable_modify_file_cap(inode: &dyn Inode, ctx: &Context) -> Result<()> {
    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        ctx.thread_local.borrow_user_ns().as_ref(),
        ctx.posix_thread,
        CapSet::SETFCAP,
    ))?;

    // With the currently supported identity user-namespace mapping, only invalid IDs are
    // unmapped. Replace this with mount-idmap-aware namespace mapping checks when supported.
    let metadata = inode.metadata();
    if metadata.uid == Uid::INVALID || metadata.gid == Gid::INVALID {
        return_errno_with_message!(
            Errno::EPERM,
            "the inode owner or group is not mapped in the current user namespace"
        );
    }

    Ok(())
}
