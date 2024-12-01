// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    device::get_device,
    fs::{
        file_table::FileDesc,
        fs_resolver::{FsPath, AT_FDCWD},
        utils::{InodeMode, InodeType, MknodType},
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

pub fn sys_mknodat(
    dirfd: FileDesc,
    path_addr: Vaddr,
    mode: u16,
    dev: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let path = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    let current = ctx.posix_thread;
    let inode_mode = {
        let mask_mode = mode & !current.fs().umask().read().get();
        InodeMode::from_bits_truncate(mask_mode)
    };
    let inode_type = InodeType::from_raw_mode(mode)?;
    debug!(
        "dirfd = {}, path = {:?}, inode_mode = {:?}, inode_type = {:?}, dev = {}",
        dirfd, path, inode_mode, inode_type, dev
    );

    let (dir_dentry, name) = {
        let path = path.to_string_lossy();
        if path.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "path is empty");
        }
        let fs_path = FsPath::new(dirfd, path.as_ref())?;
        current
            .fs()
            .resolver()
            .read()
            .lookup_dir_and_new_basename(&fs_path, false)?
    };

    match inode_type {
        InodeType::File => {
            let _ = dir_dentry.new_fs_child(&name, InodeType::File, inode_mode)?;
        }
        InodeType::CharDevice | InodeType::BlockDevice => {
            let device_inode = get_device(dev)?;
            let _ = dir_dentry.mknod(&name, inode_mode, device_inode.into())?;
        }
        InodeType::NamedPipe => {
            let _ = dir_dentry.mknod(&name, inode_mode, MknodType::NamedPipeNode)?;
        }
        InodeType::Socket => {
            return_errno_with_message!(Errno::EINVAL, "unsupported file types")
        }
        _ => return_errno_with_message!(Errno::EPERM, "unimplemented file types"),
    }

    Ok(SyscallReturn::Return(0))
}

pub fn sys_mknod(path_addr: Vaddr, mode: u16, dev: usize, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_mknodat(AT_FDCWD, path_addr, mode, dev, ctx)
}
