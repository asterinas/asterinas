// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs,
    fs::{
        file::{
            InodeMode,
            file_table::{RawFileDesc, get_file_fast},
        },
        utils::PATH_MAX,
        vfs::{
            file_system::FsFlags,
            path::{AT_FDCWD, EmptyPathStr, FsPath, Path, PerMountFlags},
        },
    },
    prelude::*,
    process::credentials::capabilities::CapSet,
    security::lsm::hooks as lsm_hooks,
};

pub(super) fn sys_fchmod(raw_fd: RawFileDesc, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    debug!("raw_fd = {}, mode = 0o{:o}", raw_fd, mode);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, raw_fd.try_into()?);
    do_chmod(file.path(), InodeMode::from_bits_truncate(mode), ctx)?;
    Ok(SyscallReturn::Return(0))
}

pub(super) fn sys_chmod(path_ptr: Vaddr, mode: u16, ctx: &Context) -> Result<SyscallReturn> {
    do_fchmodat(AT_FDCWD, path_ptr, mode, ChmodFlags::empty(), ctx)
}

pub(super) fn sys_fchmodat(
    dirfd: RawFileDesc,
    path_ptr: Vaddr,
    mode: u16,
    ctx: &Context,
) -> Result<SyscallReturn> {
    do_fchmodat(dirfd, path_ptr, mode, ChmodFlags::empty(), ctx)
}

pub(super) fn sys_fchmodat2(
    dirfd: RawFileDesc,
    path_ptr: Vaddr,
    mode: u16,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = ChmodFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid chmod flags"))?;

    do_fchmodat(dirfd, path_ptr, mode, flags, ctx)
}

fn do_fchmodat(
    dirfd: RawFileDesc,
    path_ptr: Vaddr,
    mode: u16,
    flags: ChmodFlags,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path_name = ctx.user_space().read_cstring(path_ptr, PATH_MAX)?;

    debug!(
        "dirfd = {}, path_name = {:?}, mode = 0o{:o}, flags = {:?}",
        dirfd, path_name, mode, flags,
    );

    let path = {
        let path_name = path_name.to_string_lossy();
        let fs_path =
            FsPath::from_fd_at(dirfd, &path_name, EmptyPathStr::AllowIfFlag(flags.bits()))?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let path_resolver = fs_ref.resolver().read();
        if flags.contains(ChmodFlags::AT_SYMLINK_NOFOLLOW) {
            path_resolver.lookup_no_follow(&fs_path)?
        } else {
            path_resolver.lookup(&fs_path)?
        }
    };

    do_chmod(&path, InodeMode::from_bits_truncate(mode), ctx)?;
    Ok(SyscallReturn::Return(0))
}

fn do_chmod(path: &Path, mut mode: InodeMode, ctx: &Context) -> Result<()> {
    if path.mount_node().flags().contains(PerMountFlags::RDONLY)
        || path.fs().flags().contains(FsFlags::RDONLY)
    {
        return_errno_with_message!(Errno::EROFS, "the mount or filesystem is read-only");
    }

    let metadata = path.metadata()?;
    let credentials = ctx.posix_thread.credentials();

    if credentials.fsuid() != metadata.uid {
        lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            ctx.thread_local.borrow_user_ns().as_ref(),
            ctx.posix_thread,
            CapSet::FOWNER,
        ))?;
    }

    let is_in_inode_group =
        credentials.fsgid() == metadata.gid || credentials.groups().contains(&metadata.gid);
    if mode.contains(InodeMode::S_ISGID)
        && !is_in_inode_group
        && lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
            ctx.thread_local.borrow_user_ns().as_ref(),
            ctx.posix_thread,
            CapSet::FSETID,
        ))
        .is_err()
    {
        mode.remove(InodeMode::S_ISGID);
    }

    path.set_mode(mode)?;
    fs::vfs::notify::on_attr_change(path);
    Ok(())
}

bitflags::bitflags! {
    struct ChmodFlags: u32 {
        const AT_EMPTY_PATH = 1 << 12;
        const AT_SYMLINK_NOFOLLOW = 1 << 8;
    }
}
