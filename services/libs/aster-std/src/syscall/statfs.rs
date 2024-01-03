// SPDX-License-Identifier: MPL-2.0

use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::FsPath,
    inode_handle::InodeHandle,
    utils::{SuperBlock, PATH_MAX},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::{read_cstring_from_user, write_val_to_user};

use super::SyscallReturn;
use super::{SYS_FSTATFS, SYS_STATFS};

pub fn sys_statfs(path_ptr: Vaddr, statfs_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_STATFS);
    let path = read_cstring_from_user(path_ptr, PATH_MAX)?;
    debug!("path = {:?}, statfs_buf_ptr = 0x{:x}", path, statfs_buf_ptr,);

    let current = current!();
    let dentry = {
        let path = path.to_string_lossy();
        let fs_path = FsPath::try_from(path.as_ref())?;
        current.fs().read().lookup(&fs_path)?
    };
    let statfs = Statfs::from(dentry.fs().sb());
    write_val_to_user(statfs_buf_ptr, &statfs)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_fstatfs(fd: FileDescripter, statfs_buf_ptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_FSTATFS);
    debug!("fd = {}, statfs_buf_addr = 0x{:x}", fd, statfs_buf_ptr);

    let current = current!();
    let file_table = current.file_table().lock();
    let file = file_table.get_file(fd)?;
    let inode_handle = file
        .downcast_ref::<InodeHandle>()
        .ok_or(Error::with_message(Errno::EBADF, "not inode"))?;
    let dentry = inode_handle.dentry();
    let statfs = Statfs::from(dentry.fs().sb());
    write_val_to_user(statfs_buf_ptr, &statfs)?;
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
