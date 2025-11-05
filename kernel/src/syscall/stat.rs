// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FileDesc},
        fs_resolver::{FsPath, AT_FDCWD},
        utils::Metadata,
    },
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
    time::timespec_t,
};

pub fn sys_fstat(fd: FileDesc, stat_buf_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("fd = {}, stat_buf_addr = 0x{:x}", fd, stat_buf_ptr);

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, fd);

    let stat = Stat::from(file.inode().metadata());
    ctx.user_space().write_val(stat_buf_ptr, &stat)?;

    Ok(SyscallReturn::Return(0))
}

pub fn sys_stat(filename_ptr: Vaddr, stat_buf_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_fstatat(AT_FDCWD, filename_ptr, stat_buf_ptr, 0, ctx)
}

pub fn sys_lstat(filename_ptr: Vaddr, stat_buf_ptr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    self::sys_fstatat(
        AT_FDCWD,
        filename_ptr,
        stat_buf_ptr,
        StatFlags::AT_SYMLINK_NOFOLLOW.bits(),
        ctx,
    )
}

pub fn sys_fstatat(
    dirfd: FileDesc,
    filename_ptr: Vaddr,
    stat_buf_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let filename = user_space.read_cstring(filename_ptr, MAX_FILENAME_LEN)?;
    let flags =
        StatFlags::from_bits(flags).ok_or(Error::with_message(Errno::EINVAL, "invalid flags"))?;
    debug!(
        "dirfd = {}, filename = {:?}, stat_buf_ptr = 0x{:x}, flags = {:?}",
        dirfd, filename, stat_buf_ptr, flags
    );

    if flags.contains(StatFlags::AT_EMPTY_PATH) && filename.is_empty() {
        return self::sys_fstat(dirfd, stat_buf_ptr, ctx);
    }

    let path_or_inode = {
        let filename = filename.to_string_lossy();
        let fs_path = FsPath::from_fd_and_path(dirfd, &filename)?;

        let fs_ref = ctx.thread_local.borrow_fs();
        let fs = fs_ref.resolver().read();
        if flags.contains(StatFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_inode_no_follow(&fs_path)?
        } else {
            fs.lookup_inode(&fs_path)?
        }
    };

    let stat = Stat::from(path_or_inode.inode().metadata());
    user_space.write_val(stat_buf_ptr, &stat)?;
    Ok(SyscallReturn::Return(0))
}

bitflags::bitflags! {
    struct StatFlags: u32 {
        const AT_EMPTY_PATH = 1 << 12;
        const AT_NO_AUTOMOUNT = 1 << 11;
        const AT_SYMLINK_NOFOLLOW = 1 << 8;
    }
}

/// File status; `struct stat` in Linux.
///
/// This is the x86_64-specific version.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/arch/x86/include/uapi/asm/stat.h#L83>.
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
#[cfg(target_arch = "x86_64")]
struct Stat {
    /// Device.
    st_dev: u64,
    /// File serial number.
    st_ino: u64,
    /// Link count.
    st_nlink: u64,
    /// File mode.
    st_mode: u32,
    /// User ID of the file's owner.
    st_uid: u32,
    /// Group ID of the file's group.
    st_gid: u32,
    /// Padding bytes.
    __pad0: u32,
    /// Device number, if device.
    st_rdev: u64,
    /// Total size, in bytes
    st_size: i64,
    /// Optimal block size for I/O.
    st_blksize: i64,
    /// Number 512-byte blocks allocated.
    st_blocks: i64,
    /// Time of last access.
    st_atime: timespec_t,
    /// Time of last modification.
    st_mtime: timespec_t,
    /// Time of last status change.
    st_ctime: timespec_t,
    /// Unused fields.
    __unused: [i64; 3],
}

/// File status; `struct stat` in Linux.
///
/// This is the generic version that is used by most popular 64-bit architectures except x86_64.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.16.9/source/include/uapi/asm-generic/stat.h#L24>.
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
#[cfg(not(target_arch = "x86_64"))]
struct Stat {
    /// Device.
    st_dev: u64,
    /// File serial number.
    st_ino: u64,
    /// File mode.
    st_mode: u32,
    /// Link count.
    st_nlink: u32,
    /// User ID of the file's owner.
    st_uid: u32,
    /// Group ID of the file's group.
    st_gid: u32,
    /// Device number, if device.
    st_rdev: u64,
    /// Padding bytes.
    __pad1: u64,
    /// Size of file, in bytes.
    st_size: i64,
    /// Optimal block size for I/O.
    st_blksize: i32,
    /// Padding bytes.
    __pad2: i32,
    /// Number 512-byte blocks allocated.
    st_blocks: i64,
    /// Time of last access.
    st_atime: timespec_t,
    /// Time of last modification.
    st_mtime: timespec_t,
    /// Time of last status change.
    st_ctime: timespec_t,
    /// Unused fields.
    __unused4: u32,
    /// Unused fields.
    __unused5: u32,
}

impl From<Metadata> for Stat {
    fn from(info: Metadata) -> Self {
        Self {
            st_dev: info.dev,
            st_ino: info.ino,
            st_nlink: info.nlinks as _,
            st_mode: info.type_ as u32 | info.mode.bits() as u32,
            st_uid: info.uid.into(),
            st_gid: info.gid.into(),
            st_rdev: info.rdev,
            st_size: info.size as i64,
            st_blksize: info.blk_size as _,
            st_blocks: (info.blocks * (info.blk_size / 512)) as i64,
            st_atime: info.atime.into(),
            st_mtime: info.mtime.into(),
            st_ctime: info.ctime.into(),
            ..Default::default()
        }
    }
}
