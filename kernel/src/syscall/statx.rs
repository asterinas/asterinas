// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use super::SyscallReturn;
use crate::{
    fs::{device::DeviceId, file_table::FileDesc, fs_resolver::FsPath, utils::Metadata},
    prelude::*,
    syscall::constants::MAX_FILENAME_LEN,
};

const STATX_ATTR_MOUNT_ROOT: u64 = 0x0000_2000;

pub fn sys_statx(
    dirfd: FileDesc,
    filename_ptr: Vaddr,
    flags: u32,
    mask: u32,
    statx_buf_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let filename = user_space.read_cstring(filename_ptr, MAX_FILENAME_LEN)?;
    let flags = StatxFlags::from_bits(flags)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid statx flags"))?;
    let mask = StatxMask::from_bits_truncate(mask);
    debug!(
        "dirfd = {}, filename = {:?}, flags = {:?}, mask = {:?}, statx_buf_ptr = 0x{:x}",
        dirfd, filename, flags, mask, statx_buf_ptr,
    );

    if filename.is_empty() && !flags.contains(StatxFlags::AT_EMPTY_PATH) {
        return_errno_with_message!(Errno::ENOENT, "path is empty");
    }

    if flags.contains(StatxFlags::AT_STATX_FORCE_SYNC)
        && flags.contains(StatxFlags::AT_STATX_DONT_SYNC)
    {
        return_errno_with_message!(Errno::EINVAL, "invalid statx flags");
    }

    if mask.contains(StatxMask::STATX_RESERVED) {
        return_errno_with_message!(
            Errno::EINVAL,
            "mask reserved for future struct statx expansion"
        );
    }

    let dentry = {
        let filename = filename.to_string_lossy();
        let fs_path = FsPath::new(dirfd, filename.as_ref())?;
        let fs_ref = ctx.thread_local.borrow_fs();
        let fs = fs_ref.resolver().read();
        if flags.contains(StatxFlags::AT_SYMLINK_NOFOLLOW) {
            fs.lookup_no_follow(&fs_path)?
        } else {
            fs.lookup(&fs_path)?
        }
    };

    let statx = Statx::from(dentry.metadata());

    user_space.write_val(statx_buf_ptr, &statx)?;
    Ok(SyscallReturn::Return(0))
}

/// Structures for the extended file attribute retrieval system call statx.
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct Statx {
    /// Indicates which fields in the `statx` structure were successfully filled,
    /// reflecting the state information supported by the filesystem.
    stx_mask: u32,
    /// Preferred general I/O size
    stx_blksize: u32,
    /// Flags conveying information about the file
    stx_attributes: u64,
    /// Number of hard links
    stx_nlink: u32,
    /// User ID of owner
    stx_uid: u32,
    /// Group ID of owner
    stx_gid: u32,
    /// File mode
    stx_mode: u16,
    /// Padding
    __spare0: [u16; 1],
    /// Inode number
    stx_ino: u64,
    /// File size
    stx_size: u64,
    /// Number of 512-byte blocks allocated
    stx_blocks: u64,
    /// Mask to show what's supported in stx_attributes
    stx_attributes_mask: u64,
    /// Last access time
    stx_atime: StatxTimestamp,
    /// File creation time
    stx_btime: StatxTimestamp,
    /// Last attribute change time
    stx_ctime: StatxTimestamp,
    /// Last data modification time
    stx_mtime: StatxTimestamp,
    /// Device ID of special file (if bdev/cdev)
    stx_rdev_major: u32,
    stx_rdev_minor: u32,
    /// ID of device containing file
    stx_dev_major: u32,
    stx_dev_minor: u32,
    /// Mount ID
    stx_mnt_id: u64,
    /// Memory buffer alignment for direct I/O
    stx_dio_mem_align: u32,
    /// File offset alignment for direct I/O
    stx_dio_offset_align: u32,
    /// Spare space for future expansion
    __spare3: [u64; 12],
}

impl From<Metadata> for Statx {
    fn from(info: Metadata) -> Self {
        let devid = DeviceId::from_encoded_u64(info.dev);
        let rdevid = DeviceId::from_encoded_u64(info.rdev);

        // FIXME: We assume it is always not mount_root.
        let stx_attributes = 0;

        let stx_attributes_mask = STATX_ATTR_MOUNT_ROOT;

        let stx_mask = StatxMask::STATX_TYPE.bits()
            | StatxMask::STATX_MODE.bits()
            | StatxMask::STATX_NLINK.bits()
            | StatxMask::STATX_UID.bits()
            | StatxMask::STATX_GID.bits()
            | StatxMask::STATX_ATIME.bits()
            | StatxMask::STATX_MTIME.bits()
            | StatxMask::STATX_CTIME.bits()
            | StatxMask::STATX_INO.bits()
            | StatxMask::STATX_SIZE.bits()
            | StatxMask::STATX_BLOCKS.bits()
            | StatxMask::STATX_BTIME.bits();

        Self {
            // FIXME: All zero fields below are dummy implementations that need to be improved in the future.
            stx_mask,
            stx_blksize: info.blk_size as u32,
            stx_attributes,
            stx_nlink: info.nlinks as u32,
            stx_uid: info.uid.into(),
            stx_gid: info.gid.into(),
            stx_mode: info.type_ as u16 | info.mode.bits(),
            __spare0: [0; 1],
            stx_ino: info.ino,
            stx_size: info.size as u64,
            stx_blocks: (info.blocks * (info.blk_size / 512)) as u64,
            stx_attributes_mask,
            stx_atime: StatxTimestamp::from(info.atime),
            stx_btime: StatxTimestamp::from(info.atime),
            stx_ctime: StatxTimestamp::from(info.ctime),
            stx_mtime: StatxTimestamp::from(info.ctime),
            stx_rdev_major: rdevid.major(),
            stx_rdev_minor: rdevid.minor(),
            stx_dev_major: devid.major(),
            stx_dev_minor: devid.minor(),
            stx_mnt_id: 0,
            stx_dio_mem_align: 0,
            stx_dio_offset_align: 0,
            __spare3: [0; 12],
        }
    }
}

/// Statx Timestamp (seconds and nanoseconds)
#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct StatxTimestamp {
    /// Seconds
    tv_sec: i64,
    /// Nanoseconds
    tv_nsec: u32,
    __reserved: i32,
}

impl From<Duration> for StatxTimestamp {
    fn from(duration: Duration) -> Self {
        Self {
            tv_sec: duration.as_secs() as i64,
            tv_nsec: duration.subsec_nanos(),
            __reserved: 0,
        }
    }
}

bitflags! {
    /// Flags can be used to influence a pathname-based lookup.
    /// Flags can also be used to control what sort of synchronization the
    /// kernel will do when querying a file on a remote filesystem.
    struct StatxFlags: u32 {
        /// Allow empty relative pathname to operate on dirfd directly.
        const AT_EMPTY_PATH         = 1 << 12;
        /// Suppress terminal automount traversal.
        const AT_NO_AUTOMOUNT       = 1 << 11;
        /// Do not follow symbolic links.
        const AT_SYMLINK_NOFOLLOW   = 1 << 8;
        /// Do whatever stat() does.
        const AT_STATX_SYNC_AS_STAT = 0;
        /// Force the attributes to be sync'd with the server.
        const AT_STATX_FORCE_SYNC   = 1 << 13;
        /// Don't sync attributes with the server.
        const AT_STATX_DONT_SYNC    = 1 << 14;
    }
}

bitflags! {
    /// Flags to be stx_mask.
    /// Query request/result mask for statx() and struct statx::stx_mask.
    /// These bits should be set in the mask argument of statx() to request
    /// particular items when calling statx().
    pub struct StatxMask: u32 {
        /// Want stx_mode & S_IFMT
        const STATX_TYPE            = 0x00000001;
        /// Want stx_mode & ~S_IFMT
        const STATX_MODE            = 0x00000002;
        /// Want stx_nlink
        const STATX_NLINK           = 0x00000004;
        /// Want stx_uid
        const STATX_UID             = 0x00000008;
        /// Want stx_gid
        const STATX_GID             = 0x00000010;
        /// Want stx_atime
        const STATX_ATIME           = 0x00000020;
        /// Want stx_mtime
        const STATX_MTIME           = 0x00000040;
        /// Want stx_ctime
        const STATX_CTIME           = 0x00000080;
        /// Want stx_ino
        const STATX_INO             = 0x00000100;
        /// Want stx_size
        const STATX_SIZE            = 0x00000200;
        /// Want stx_blocks
        const STATX_BLOCKS          = 0x00000400;
        /// All of the above (stx_mode, stx_nlink, etc.)
        const STATX_BASIC_STATS     = 0x000007ff;
        /// Want stx_btime
        const STATX_BTIME           = 0x00000800;
        /// Deprecated: The same as STATX_BASIC_STATS | STATX_BTIME
        const STATX_ALL             = 0x00000fff;
        /// Want stx_mnt_id
        const STATX_MNT_ID          = 0x00001000;
        /// Want stx_dio_mem_align and stx_dio_offset_align
        const STATX_DIOALIGN        = 0x00002000;
        /// Reserved for future struct statx expansion
        const STATX_RESERVED		= 0x80000000;
        /// Want/got stx_change_attr
        const STATX_CHANGE_COOKIE   = 0x40000000;
    }
}
