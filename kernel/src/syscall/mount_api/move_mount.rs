// SPDX-License-Identifier: MPL-2.0

use super::super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    fs::{
        file::{
            DetachedMountFile,
            file_table::{RawFileDesc, get_file_fast},
        },
        vfs::path::{EmptyPathStr, FsPath, Mount, Path},
    },
    prelude::*,
};

pub fn sys_move_mount(
    from_dfd: RawFileDesc,
    from_path_addr: Vaddr,
    to_dfd: RawFileDesc,
    to_path_addr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = MoveMountFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown move_mount flags"))?;

    let from_path = ctx
        .user_space()
        .read_cstring(from_path_addr, MAX_FILENAME_LEN)?;
    let source = if from_path.is_empty() {
        if !flags.contains(MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH) {
            return_errno_with_message!(Errno::ENOENT, "the source path is empty");
        }
        MoveMountSource::Detached(get_detached_mount(from_dfd, ctx)?)
    } else {
        let from_path = from_path.to_string_lossy();
        MoveMountSource::Path(lookup_path(from_dfd, &from_path, ctx)?)
    };

    let to_path = ctx
        .user_space()
        .read_cstring(to_path_addr, MAX_FILENAME_LEN)?;
    let to_path = to_path.to_string_lossy();
    let target_path = lookup_path(to_dfd, &to_path, ctx)?;

    match source {
        MoveMountSource::Detached(detached_mount) => {
            target_path.attach_detached_mount(&detached_mount, ctx)?;
        }
        MoveMountSource::Path(source_path) => source_path.move_mount_to(&target_path, ctx)?,
    };

    Ok(SyscallReturn::Return(0))
}

enum MoveMountSource {
    Detached(Arc<Mount>),
    Path(Path),
}

fn lookup_path(dirfd: RawFileDesc, path: &str, ctx: &Context) -> Result<Path> {
    let fs_path = FsPath::from_fd_at(dirfd, path, EmptyPathStr::Reject)?;
    ctx.thread_local
        .borrow_fs()
        .resolver()
        .read()
        .lookup(&fs_path)
}

fn get_detached_mount(from_dfd: RawFileDesc, ctx: &Context) -> Result<Arc<Mount>> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, from_dfd.try_into()?);
    let mount_file = file
        .downcast_ref::<DetachedMountFile>()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "the file is not a detached mount"))?;

    Ok(mount_file.mount())
}

bitflags! {
    struct MoveMountFlags: u32 {
        const MOVE_MOUNT_F_EMPTY_PATH = 0x0000_0004;
    }
}
