// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{FdFlags, FileDesc, get_file_fast},
        notify::inotify::{InotifyControls, InotifyEvents, InotifyFile},
        path::FsPath,
        utils::{InodeType, Permission},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_inotify_init(ctx: &Context) -> Result<SyscallReturn> {
    do_inotify_init(0, ctx)
}

pub fn sys_inotify_init1(flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    do_inotify_init(flags, ctx)
}

fn do_inotify_init(flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("inotify_init flags = {}", flags);
    let flags = InotifyFileFlags::from_bits(flags)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    let fd_flags = if flags.contains(InotifyFileFlags::CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };
    let is_nonblocking = flags.contains(InotifyFileFlags::NONBLOCK);
    let file = InotifyFile::new(is_nonblocking)?;
    let file_table = ctx.thread_local.borrow_file_table();
    let fd = file_table.unwrap().write().insert(file, fd_flags);
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_inotify_add_watch(
    fd: FileDesc,
    path: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("fd = {:?}, path = {:?}, flags = {}", fd, path, flags);
    if flags == 0 {
        return_errno_with_message!(Errno::EINVAL, "flags is 0, no events to watch");
    }
    // Parse flags to InotifyEvents.
    let (interesting, options) = parse_inotify_watch_request(flags)?;

    if options.contains(InotifyControls::MASK_ADD) && options.contains(InotifyControls::MASK_CREATE)
    {
        return_errno_with_message!(Errno::EINVAL, "flags is invalid");
    }

    let path = ctx.user_space().read_cstring(path, MAX_FILENAME_LEN)?;
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    // Verify that the file is an inotify file.
    let inotify_file = match file.downcast_ref::<InotifyFile>() {
        Some(inotify_file) => inotify_file,
        None => return_errno_with_message!(Errno::EINVAL, "file is not an inotify file"),
    };

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::try_from(path.as_ref())?;

        if options.contains(InotifyControls::DONT_FOLLOW) {
            ctx.thread_local
                .borrow_fs()
                .resolver()
                .read()
                .lookup_no_follow(&fs_path)?
        } else {
            ctx.thread_local
                .borrow_fs()
                .resolver()
                .read()
                .lookup(&fs_path)?
        }
    };

    // Verify caller has read permissions on the inode.
    let inode = dentry.inode();
    inode.check_permission(Permission::MAY_READ)?;

    if options.contains(InotifyControls::ONLYDIR) && inode.type_() != InodeType::Dir {
        return_errno_with_message!(Errno::ENOTDIR, "path is not a directory");
    }

    let wd = inotify_file.add_watch(&dentry, interesting, options)?;
    Ok(SyscallReturn::Return(wd as _))
}

pub fn sys_inotify_rm_watch(fd: FileDesc, wd: u32, ctx: &Context) -> Result<SyscallReturn> {
    debug!("inotify_rm_watch fd = {}, wd = {}", fd, wd);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    let inotify_file = match file.downcast_ref::<InotifyFile>() {
        Some(inotify_file) => inotify_file,
        None => return_errno_with_message!(Errno::EINVAL, "file is not an inotify file"),
    };
    inotify_file.remove_watch(wd)?;
    Ok(SyscallReturn::Return(0))
}

fn parse_inotify_watch_request(flags: u32) -> Result<(InotifyEvents, InotifyControls)> {
    let interesting = InotifyEvents::from_bits_truncate(flags);
    let options = InotifyControls::from_bits_truncate(flags);
    let recognized_bits = interesting.bits() | options.bits();

    if flags & !recognized_bits != 0 {
        return_errno_with_message!(Errno::EINVAL, "invalid flags");
    }

    Ok((interesting, options))
}

bitflags! {
    struct InotifyFileFlags: u32 {
        const NONBLOCK = 1 << 11; // Non-blocking
        const CLOEXEC = 1 << 19; // Close on exec
    }
}
