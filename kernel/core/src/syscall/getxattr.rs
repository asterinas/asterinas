// SPDX-License-Identifier: MPL-2.0

use super::{
    SyscallReturn,
    setxattr::{
        XattrFileCtx, check_xattr_namespace, lookup_path_for_xattr, parse_xattr_name,
        read_xattr_name_cstr_from_user,
    },
};
use crate::{
    fs::{
        file::file_table::{RawFileDesc, get_file_fast},
        vfs::xattr::{self, XATTR_VALUE_MAX_LEN},
    },
    prelude::*,
    process::credentials::{FileCapabilities, Uid, VfsCapRevision},
    syscall::constants::MAX_FILENAME_LEN,
};

pub(super) fn sys_getxattr(
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

pub(super) fn sys_lgetxattr(
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

pub(super) fn sys_fgetxattr(
    raw_fd: RawFileDesc,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);

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

    let path = lookup_path_for_xattr(&file_ctx, ctx)?;
    if xattr_name.full_name() == xattr::SECURITY_CAPABILITY_XATTR_NAME {
        let mut raw_value = [0u8; FileCapabilities::MAX_XATTR_SIZE];
        let mut raw_value_writer = VmWriter::from(raw_value.as_mut_slice()).to_fallible();
        let raw_value_len = path.get_xattr(xattr_name, &mut raw_value_writer)?;
        let converted_len = convert_file_cap(&mut raw_value[..raw_value_len])?;

        if value_len == 0 {
            // TODO: In general, if the user-provided buffer length is zero, return the
            // attribute's length without copying its contents to userspace. Currently, this
            // behavior is only handled for `security.capability`; other attributes are not yet
            // supported.
            return Ok(converted_len);
        }
        if value_len < converted_len {
            return_errno_with_message!(Errno::ERANGE, "the xattr value buffer is too small");
        }

        let mut value_writer = user_space.writer(value_ptr, converted_len)?;
        value_writer.write_fallible(&mut VmReader::from(&raw_value[..converted_len]))?;
        return Ok(converted_len);
    }

    let mut value_writer = user_space.writer(value_ptr, value_len.min(XATTR_VALUE_MAX_LEN))?;
    path.get_xattr(xattr_name, &mut value_writer)
}

/// Converts a file-capability xattr under the currently supported identity
/// user-namespace mapping.
///
/// FIXME: Apply filesystem-user-namespace and ID-mapped-mount UID mappings when
/// those features are supported.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/security/commoncap.c#L420>.
fn convert_file_cap(raw_value: &mut [u8]) -> Result<usize> {
    let (revision, flags) = FileCapabilities::parse_header(raw_value)?;

    match revision {
        VfsCapRevision::V1 => {
            return_errno_with_message!(Errno::EINVAL, "v1 file capabilities cannot be read")
        }
        VfsCapRevision::V2 => Ok(raw_value.len()),
        VfsCapRevision::V3 => {
            let root_uid = FileCapabilities::read_v3_uid(raw_value)?;
            if root_uid == Uid::INVALID {
                return_errno_with_message!(
                    Errno::EOVERFLOW,
                    "v3 file capabilities contain an invalid root UID"
                );
            }
            if root_uid.is_root() {
                let converted_header = VfsCapRevision::V2 as u32 | flags.bits();
                raw_value[..size_of::<u32>()].copy_from_slice(&converted_header.to_le_bytes());
                Ok(VfsCapRevision::V2.xattr_size())
            } else {
                Ok(raw_value.len())
            }
        }
    }
}
