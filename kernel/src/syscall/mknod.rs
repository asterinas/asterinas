// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        self,
        file_table::FileDesc,
        path::{AT_FDCWD, FsPath},
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
    let path_name = ctx.user_space().read_cstring(path_addr, MAX_FILENAME_LEN)?;
    let fs_ref = ctx.thread_local.borrow_fs();
    let inode_mode = {
        let mask_mode = mode & !fs_ref.umask().get();
        InodeMode::from_bits_truncate(mask_mode)
    };
    let inode_type = InodeType::from_raw_mode(mode)?;
    debug!(
        "dirfd = {}, path = {:?}, inode_mode = {:?}, inode_type = {:?}, dev = {}",
        dirfd, path_name, inode_mode, inode_type, dev
    );

    let (dir_path, name) = {
        let path_name = path_name.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(dirfd, &path_name)?;
        fs_ref
            .resolver()
            .read()
            .lookup_unresolved_no_follow(&fs_path)?
            .into_parent_and_filename()?
    };

    match inode_type {
        InodeType::File => {
            let _ = dir_path.new_fs_child(&name, InodeType::File, inode_mode)?;
        }
        InodeType::CharDevice => {
            let _ = dir_path.mknod(&name, inode_mode, MknodType::CharDevice(dev as u64))?;
        }
        InodeType::BlockDevice => {
            let _ = dir_path.mknod(&name, inode_mode, MknodType::BlockDevice(dev as u64))?;
        }
        InodeType::NamedPipe => {
            let _ = dir_path.mknod(&name, inode_mode, MknodType::NamedPipe)?;
        }
        InodeType::Socket => {
            return_errno_with_message!(Errno::EINVAL, "unsupported file types")
        }
        _ => return_errno_with_message!(Errno::EPERM, "unimplemented file types"),
    }
    fs::notify::on_create(&dir_path, || name);
    Ok(SyscallReturn::Return(0))
}

pub fn sys_mknod(path_addr: Vaddr, mode: u16, dev: usize, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_mknodat(AT_FDCWD, path_addr, mode, dev, ctx)
}
