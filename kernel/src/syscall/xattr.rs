// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        utils::{
            XattrFlags, XattrNamespace, XATTR_LIST_MAX_LEN, XATTR_NAME_MAX_LEN, XATTR_VALUE_MAX_LEN,
        },
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
    syscall::constants::MAX_FILENAME_LEN,
};

///////////////// setxattr

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

    let flags = XattrFlags::from_bits(flags as _)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid xattr flags"))?;

    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?;
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }

    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx)?;

    // TODO: Leave it to fs
    if value_len > XATTR_VALUE_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr value too long"));
    }
    let mut value_reader = user_space.reader(value_ptr, value_len)?;

    debug!("setxattr path = {path:?}, name: {name:?}, namespace: {namespace:?}, value_len: {value_len}, flags = {flags:?}");

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup(&fs_path)?
    };
    dentry.set_xattr(namespace, name.as_ref(), &mut value_reader, flags)?;

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

    let flags = XattrFlags::from_bits(flags as _)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid xattr flags"))?;

    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?;
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }
    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx)?;

    // TODO: Leave it to fs
    if value_len > XATTR_VALUE_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr value too long"));
    }
    let mut value_reader = user_space.reader(value_ptr, value_len)?;

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup_no_follow(&fs_path)?
    };
    dentry.set_xattr(namespace, name.as_ref(), &mut value_reader, flags)?;

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

    let flags = XattrFlags::from_bits(flags as _)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid xattr flags"))?;

    let user_space = ctx.user_space();
    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?;
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }
    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx)?;

    // TODO: Leave it to fs
    if value_len > XATTR_VALUE_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr value too long"));
    }
    let mut value_reader = user_space.reader(value_ptr, value_len)?;

    let dentry = file.as_inode_or_err()?.dentry();
    dentry.set_xattr(namespace, name.as_ref(), &mut value_reader, flags)?;

    Ok(SyscallReturn::Return(0))
}

///////////////// getxattr

pub fn sys_getxattr(
    path_ptr: Vaddr,
    name_ptr: Vaddr,
    value_ptr: Vaddr,
    value_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?; // or not + 1
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }
    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx).map_err(|_| Error::new(Errno::ENODATA))?;

    // if value_len > XATTR_VALUE_MAX_LEN {
    //     return Err(Error::with_message(Errno::E2BIG, "xattr value too long"));
    // }
    let mut value_writer = user_space.writer(value_ptr, value_len.min(XATTR_VALUE_MAX_LEN))?;

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup(&fs_path)?
    };
    let len = dentry.get_xattr(namespace, name.as_ref(), &mut value_writer)?;

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
    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?; // or not + 1
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }
    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx).map_err(|_| Error::new(Errno::ENODATA))?;

    if value_len > XATTR_VALUE_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr value too long"));
    }
    let mut value_writer = user_space.writer(value_ptr, value_len)?;

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup_no_follow(&fs_path)?
    };
    let len = dentry.get_xattr(namespace, name.as_ref(), &mut value_writer)?;

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
    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?; // or not + 1
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }
    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx).map_err(|_| Error::new(Errno::ENODATA))?;

    if value_len > XATTR_VALUE_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr value too long"));
    }
    let mut value_writer = user_space.writer(value_ptr, value_len)?;

    let dentry = file.as_inode_or_err()?.dentry();
    let len = dentry.get_xattr(namespace, name.as_ref(), &mut value_writer)?;

    Ok(SyscallReturn::Return(len as _))
}

///////////////// listxattr

pub fn sys_listxattr(
    path_ptr: Vaddr,
    // name list
    list_ptr: Vaddr,
    list_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    if list_len > XATTR_LIST_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr list too long"));
    }
    let mut list_writer = user_space.writer(list_ptr, list_len)?;

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup(&fs_path)?
    };
    let len = dentry.list_xattr(&mut list_writer)?;

    Ok(SyscallReturn::Return(len as _))
}

pub fn sys_llistxattr(
    path_ptr: Vaddr,
    // name list
    list_ptr: Vaddr,
    list_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;
    if list_len > XATTR_LIST_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr list too long"));
    }
    let mut list_writer = user_space.writer(list_ptr, list_len)?;

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup_no_follow(&fs_path)?
    };
    let len = dentry.list_xattr(&mut list_writer)?;

    Ok(SyscallReturn::Return(len as _))
}

pub fn sys_flistxattr(
    fd: FileDesc,
    // name list
    list_ptr: Vaddr,
    list_len: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.file_table().borrow_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let user_space = ctx.user_space();
    if list_len > XATTR_LIST_MAX_LEN {
        return Err(Error::with_message(Errno::E2BIG, "xattr list too long"));
    }
    //
    let mut list_writer = user_space.writer(list_ptr, list_len)?;

    let dentry = file.as_inode_or_err()?.dentry();
    let len = dentry.list_xattr(&mut list_writer)?;

    Ok(SyscallReturn::Return(len as _))
}

///////////////// removexattr

pub fn sys_removexattr(path_ptr: Vaddr, name_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?;
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }

    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx)?;

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup(&fs_path)?
    };
    dentry.remove_xattr(namespace, name.as_ref())?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_lremovexattr(path_ptr: Vaddr, name_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, MAX_FILENAME_LEN)?;

    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?;
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }

    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx)?;

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::new(AT_FDCWD, path.as_ref())?;
        let fs = ctx.posix_thread.fs().resolver().read();
        fs.lookup_no_follow(&fs_path)?
    };
    dentry.remove_xattr(namespace, name.as_ref())?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_fremovexattr(fd: FileDesc, name_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.file_table().borrow_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let user_space = ctx.user_space();

    let name = user_space.read_cstring(name_ptr, XATTR_NAME_MAX_LEN + 1)?;
    let name = name.to_string_lossy();
    if name.is_empty() || name.len() > XATTR_NAME_MAX_LEN {
        return Err(Error::with_message(
            Errno::ERANGE,
            "xattr name empty or too long",
        ));
    }

    let namespace = XattrNamespace::try_from_name(name.as_ref()).ok_or(Error::with_message(
        Errno::EOPNOTSUPP,
        "invalid xattr namespace",
    ))?;
    check_xattr_namespace(namespace, ctx)?;

    let dentry = file.as_inode_or_err()?.dentry();
    dentry.remove_xattr(namespace, name.as_ref())?;

    Ok(SyscallReturn::Return(0))
}

fn check_xattr_namespace(namespace: XattrNamespace, ctx: &Context) -> Result<()> {
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
