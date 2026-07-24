// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, constants::MAX_FILENAME_LEN};
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
    let flags = MoveMountFlags::try_from(flags)?;
    let supported_flags = MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH;
    if !(flags - supported_flags).is_empty() {
        return_errno_with_message!(Errno::EOPNOTSUPP, "unsupported move_mount flags");
    }
    super::fsopen::check_mount_api_capability(ctx)?;

    let from_path = ctx
        .user_space()
        .read_cstring(from_path_addr, MAX_FILENAME_LEN)?;
    let source = if from_path.is_empty() {
        if !flags.contains(MoveMountFlags::MOVE_MOUNT_F_EMPTY_PATH) {
            return_errno_with_message!(Errno::ENOENT, "the source path is empty");
        }
        MoveMountSource::Detached(get_detached_mount(from_dfd, ctx)?)
    } else {
        MoveMountSource::Path(from_path)
    };

    let to_path = ctx
        .user_space()
        .read_cstring(to_path_addr, MAX_FILENAME_LEN)?;

    let fs_ref = ctx.thread_local.borrow_fs();
    let path_resolver = fs_ref.resolver().read();
    let to_path = to_path.to_string_lossy();
    let to_fs_path = FsPath::from_fd_at(to_dfd, &to_path, EmptyPathStr::Reject)?;
    let target_path = path_resolver.lookup(&to_fs_path)?;
    match source {
        MoveMountSource::Detached(detached_mount) => {
            let detached_root = Path::new_fs_root(detached_mount);
            detached_root.move_mount_to(target_path, ctx)?;
        }
        MoveMountSource::Path(from_path) => {
            let from_path = from_path.to_string_lossy();
            let from_fs_path = FsPath::from_fd_at(from_dfd, &from_path, EmptyPathStr::Reject)?;
            let source_path = path_resolver.lookup(&from_fs_path)?;
            source_path.move_mount_to(target_path, ctx)?;
        }
    };

    Ok(SyscallReturn::Return(0))
}

enum MoveMountSource {
    Detached(Arc<Mount>),
    Path(CString),
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
        const MOVE_MOUNT_F_SYMLINKS   = 0x0000_0001;
        const MOVE_MOUNT_F_AUTOMOUNTS = 0x0000_0002;
        const MOVE_MOUNT_F_EMPTY_PATH = 0x0000_0004;
        const MOVE_MOUNT_T_SYMLINKS   = 0x0000_0010;
        const MOVE_MOUNT_T_AUTOMOUNTS = 0x0000_0020;
        const MOVE_MOUNT_T_EMPTY_PATH = 0x0000_0040;
        const MOVE_MOUNT_SET_GROUP    = 0x0000_0100;
        const MOVE_MOUNT_BENEATH      = 0x0000_0200;
    }
}

impl TryFrom<u32> for MoveMountFlags {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        Self::from_bits(value)
            .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown move_mount flags"))
    }
}
