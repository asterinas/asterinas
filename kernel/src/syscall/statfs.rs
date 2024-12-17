// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::FileDesc,
        fs_resolver::FsPath,
        inode_handle::InodeHandle,
        utils::{SuperBlock, PATH_MAX},
    },
    prelude::*,
};

pub fn sys_statfs(path_ptr: Vaddr, statfs_buf_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let path = user_space.read_cstring(path_ptr, PATH_MAX)?;
    debug!("path = {:?}, statfs_buf_ptr = 0x{:x}", path, statfs_buf_ptr,);

    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::try_from(path.as_ref())?;
        ctx.posix_thread.fs().resolver().read().lookup(&fs_path)?
    };
    let statfs = Statfs::from(dentry.fs().sb());
    user_space.write_val(statfs_buf_ptr, &statfs)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fstatfs(fd: FileDesc, statfs_buf_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}, statfs_buf_addr = 0x{:x}", fd, statfs_buf_ptr);

    let fs = {
        let file_table = ctx.posix_thread.file_table().lock();
        let file = file_table.get_file(fd)?;
        let inode_handle = file
            .downcast_ref::<InodeHandle>()
            .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
        inode_handle.dentry().fs()
    };

    let statfs = Statfs::from(fs.sb());
    ctx.user_space().write_val(statfs_buf_ptr, &statfs)?;
    Ok(SyscallReturn::Return(0))
}

/// FS Stat
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
struct Statfs {
    /// Type of filesystem
    f_type: u64,
    /// Optimal transfer block size
    f_bsize: usize,
    /// Total data blocks in filesystem
    f_blocks: usize,
    /// Free blocks in filesystem
    f_bfree: usize,
    /// Free blocks available to unprivileged user
    f_bavail: usize,
    /// Total inodes in filesystem
    f_files: usize,
    /// Free inodes in filesystem
    f_ffree: usize,
    /// Filesystem ID
    f_fsid: u64,
    /// Maximum length of filenames
    f_namelen: usize,
    /// Fragment size
    f_frsize: usize,
    /// Mount flags of filesystem
    f_flags: u64,
    /// Padding bytes reserved for future use
    f_spare: [u64; 4],
}

impl From<SuperBlock> for Statfs {
    fn from(sb: SuperBlock) -> Self {
        Self {
            f_type: sb.magic,
            f_bsize: sb.bsize,
            f_blocks: sb.blocks,
            f_bfree: sb.bfree,
            f_bavail: sb.bavail,
            f_files: sb.files,
            f_ffree: sb.ffree,
            f_fsid: sb.fsid,
            f_namelen: sb.namelen,
            f_frsize: sb.frsize,
            f_flags: sb.flags,
            f_spare: [0u64; 4],
        }
    }
}
