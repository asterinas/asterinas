// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FdFlags, FileDesc},
        fs_resolver::FsPath,
        notify::inotify::{InotifyFile, InotifyFlags, InotifyMask},
        utils::InodeType,
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
    let flags = InotifyFlags::from_bits(flags)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    let file = InotifyFile::new(flags);
    let file_table = ctx.thread_local.borrow_file_table();
    let fd = file_table.unwrap().write().insert(file, FdFlags::empty());
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_inotify_add_watch(
    fd: FileDesc,
    path: Vaddr,
    mask: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path, MAX_FILENAME_LEN)?;
    debug!("fd = {:?}, path = {:?}, mask = {}", fd, path, mask);
    if mask == 0 {
        return_errno_with_message!(Errno::EINVAL, "mask is 0, no events to watch");
    }
    let mask =
        InotifyMask::from_bits(mask).ok_or(Error::with_message(Errno::EINVAL, "invalid mask"))?;

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);
    if mask.contains(InotifyMask::IN_MASK_ADD) && mask.contains(InotifyMask::IN_MASK_CREATE) {
        return_errno_with_message!(Errno::EINVAL, "flags is invalid");
    }

    let inotify_file = match file.downcast_ref::<InotifyFile>() {
        Some(inotify_file) => inotify_file,
        None => return_errno_with_message!(Errno::EINVAL, "file is not an inotify file"),
    };

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::try_from(path.as_ref())?;

        if mask.contains(InotifyMask::IN_DONT_FOLLOW) {
            ctx.posix_thread
                .fs()
                .resolver()
                .read()
                .lookup_no_follow(&fs_path)?
        } else {
            ctx.posix_thread.fs().resolver().read().lookup(&fs_path)?
        }
    };

    if mask.contains(InotifyMask::IN_ONLYDIR) {
        if dentry.inode().type_() != InodeType::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "path is not a directory");
        }
    }

    let wd = inotify_file.update_watch(&dentry, mask.bits())?;
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
    inotify_file.inotify_remove_watch(wd)?;
    Ok(SyscallReturn::Return(0))
}
