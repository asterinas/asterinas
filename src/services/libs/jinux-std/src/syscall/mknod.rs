use crate::fs::{
    file_table::FileDescripter,
    fs_resolver::{FsPath, AT_FDCWD},
    utils::{DeviceId, InodeMode, InodeType},
};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::syscall::constants::MAX_FILENAME_LEN;
use crate::util::read_cstring_from_user;

use super::SyscallReturn;
use super::SYS_MKNODAT;

pub fn sys_mknodat(
    dirfd: FileDescripter,
    pathname_addr: Vaddr,
    mode: u32,
    dev: u64,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MKNODAT);
    let pathname = read_cstring_from_user(pathname_addr, MAX_FILENAME_LEN)?;
    debug!(
        "dirfd = {}, pathname = {:?}, mode = 0o{:o}, dev = {}",
        dirfd, pathname, mode, dev,
    );

    let inode_type = InodeType::from_mode(mode)?;
    let inode_mode = {
        const MODE_MASK: u32 = 0o7777;
        let mode = (mode & MODE_MASK) as u16;
        InodeMode::from_bits_truncate(mode)
    };
    let dev = if inode_type == InodeType::CharDevice || inode_type == InodeType::BlockDevice {
        Some(DeviceId::from(dev as usize))
    } else {
        None
    };

    let current = current!();
    let (dir_dentry, name) = {
        let pathname = pathname.to_string_lossy();
        if pathname.is_empty() {
            return_errno_with_message!(Errno::ENOENT, "pathname is empty");
        }
        if pathname.ends_with("/") {
            return_errno_with_message!(Errno::EPERM, "pathname is dir");
        }
        let fs_path = FsPath::new(dirfd, pathname.as_ref())?;
        current.fs().read().lookup_dir_and_base_name(&fs_path)?
    };
    let _ = dir_dentry.mknod(&name, inode_type, inode_mode, dev)?;
    Ok(SyscallReturn::Return(0))
}

pub fn sys_mknod(pathname_addr: Vaddr, mode: u32, dev: u64) -> Result<SyscallReturn> {
    self::sys_mknodat(AT_FDCWD, pathname_addr, mode, dev)
}

trait MknodExt: Sized {
    fn from_mode(mode: u32) -> Result<Self>;
}

impl MknodExt for InodeType {
    fn from_mode(mode: u32) -> Result<Self> {
        const TYPE_MASK: u32 = 0o170000;
        let bits = mode & TYPE_MASK;
        let inode_type = if bits == Self::NamedPipe as u32 {
            Self::NamedPipe
        } else if bits == Self::CharDevice as u32 {
            Self::CharDevice
        } else if bits == Self::BlockDevice as u32 {
            Self::BlockDevice
        } else if bits == Self::File as u32 {
            Self::File
        } else if bits == Self::Socket as u32 {
            Self::Socket
        } else if bits == 0 {
            // Zero file type is equivalent to type File
            Self::File
        } else {
            return Err(Error::with_message(Errno::EINVAL, "invalid mode"));
        };
        Ok(inode_type)
    }
}
