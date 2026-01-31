// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use alloc::boxed::ThinBox;
use core::time::Duration;

use core2::io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult, Write};
use device_id::DeviceId;
use ostd::task::Task;
use spin::Once;

use super::{
    AccessMode, DirentVisitor, FallocMode, FileSystem, InodeMode, XattrName, XattrNamespace,
    XattrSetFlags,
};
use crate::{
    fs::{
        device::{Device, DeviceType},
        inode_handle::FileIo,
        path::Path,
        utils::StatusFlags,
    },
    prelude::*,
    process::{Gid, Uid, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    time::clocks::RealTimeCoarseClock,
    vm::vmo::Vmo,
};

#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum InodeType {
    Unknown = 0o000000,
    NamedPipe = 0o010000,
    CharDevice = 0o020000,
    Dir = 0o040000,
    BlockDevice = 0o060000,
    File = 0o100000,
    SymLink = 0o120000,
    Socket = 0o140000,
}

impl InodeType {
    pub fn is_regular_file(&self) -> bool {
        *self == InodeType::File
    }

    pub fn is_directory(&self) -> bool {
        *self == InodeType::Dir
    }

    pub fn is_device(&self) -> bool {
        *self == InodeType::BlockDevice || *self == InodeType::CharDevice
    }

    pub fn is_seekable(&self) -> bool {
        *self != InodeType::NamedPipe && *self != Self::Socket
    }

    /// Parse the inode type in the `mode` from syscall, and convert it into `InodeType`.
    pub fn from_raw_mode(mut mode: u16) -> Result<Self> {
        const TYPE_MASK: u16 = 0o170000;
        mode &= TYPE_MASK;

        // Special case
        if mode == 0 {
            return Ok(Self::File);
        }
        Self::try_from(mode & TYPE_MASK)
            .map_err(|_| Error::with_message(Errno::EINVAL, "invalid file type"))
    }

    pub fn device_type(&self) -> Option<DeviceType> {
        match self {
            InodeType::BlockDevice => Some(DeviceType::Block),
            InodeType::CharDevice => Some(DeviceType::Char),
            _ => None,
        }
    }
}

impl From<DeviceType> for InodeType {
    fn from(type_: DeviceType) -> InodeType {
        match type_ {
            DeviceType::Char => InodeType::CharDevice,
            DeviceType::Block => InodeType::BlockDevice,
        }
    }
}

bitflags! {
    pub struct Permission: u16 {
        // This implementation refers the implementation of linux
        // https://elixir.bootlin.com/linux/v6.0.9/source/include/linux/fs.h#L95
        const MAY_EXEC		= 0x0001;
        const MAY_WRITE		= 0x0002;
        const MAY_READ		= 0x0004;
        const MAY_APPEND    = 0x0008;
        const MAY_ACCESS	= 0x0010;
        const MAY_OPEN		= 0x0020;
        const MAY_CHDIR		= 0x0040;
        const MAY_NOT_BLOCK	= 0x0080;
    }
}
impl Permission {
    pub fn may_read(&self) -> bool {
        self.contains(Self::MAY_READ)
    }

    pub fn may_write(&self) -> bool {
        self.contains(Self::MAY_WRITE)
    }

    pub fn may_exec(&self) -> bool {
        self.contains(Self::MAY_EXEC)
    }
}
impl From<AccessMode> for Permission {
    fn from(access_mode: AccessMode) -> Permission {
        match access_mode {
            AccessMode::O_RDONLY => Permission::MAY_READ,
            AccessMode::O_WRONLY => Permission::MAY_WRITE,
            AccessMode::O_RDWR => Permission::MAY_READ | Permission::MAY_WRITE,
        }
    }
}

/// File metadata, providing detailed information about an inode.
///
/// Asterinas's type-safe counterparts for Linux's `struct stat`.
#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    /// The inode number, which uniquely identifies the file within the filesystem.
    ///
    /// Corresponds to `st_ino`.
    pub ino: u64,

    /// The size of the inode.
    ///
    /// The interpretation depends on `inode_type`:
    /// - **Regular File**: The total length of the file content.
    /// - **Directory**: The size of the directory's internal table (usually a multiple of block size).
    /// - **Symbolic Link**: The length of the target pathname.
    /// - **Device/Socket/FIFO**: Usually zero.
    ///
    /// Corresponds to `st_size`.
    pub size: usize,

    /// The optimal block size for filesystem I/O operations.
    ///
    /// Corresponds to `st_blksize`.
    pub optimal_block_size: usize,

    /// The number of 512-byte sectors allocated for the inode on disk.
    ///
    /// This represents physical usage.
    /// For sparse files (those having holes), `size` is greater than this field.
    /// For files with preallocated blocks (`FALLOC_FL_KEEP_SIZE`),
    /// `size` is smaller than this field.
    ///
    /// Corresponds to `st_blocks`.
    pub nr_sectors_allocated: usize,

    /// The timestamp of the last access to the inode's data.
    ///
    /// Corresponds to `st_atime`.
    pub last_access_at: Duration,

    /// The timestamp of the last modification to the inode's content.
    ///
    /// Corresponds to `st_mtime`.
    pub last_modify_at: Duration,

    /// The timestamp of the last change to the inode's metadata.
    ///
    /// This is updated when permissions, ownership, or link count change,
    /// not just when the inode content is modified.
    ///
    /// Corresponds to `st_ctime`.
    pub last_meta_change_at: Duration,

    /// The type of the inode (e.g., regular file, directory, symlink).
    ///
    /// Derived from the file type bits of `st_mode` (using the `S_IFMT` mask).
    pub type_: InodeType,

    /// The inode mode, representing access permissions.
    ///
    /// Derived from the permission bits of `st_mode`.
    pub mode: InodeMode,

    /// The number of hard links pointing to this inode.
    ///
    /// Corresponds to `st_nlink`.
    pub nr_hard_links: usize,

    /// The User ID (UID) of the inode's owner.
    ///
    /// Corresponds to `st_uid`.
    pub uid: Uid,

    /// The Group ID (GID) of the inode's owner.
    ///
    /// Corresponds to `st_gid`.
    pub gid: Gid,

    /// The ID of the device containing the inode.
    ///
    /// For persisted files, this device could be a on-disk partition
    /// or a logical volume (with RAID).
    /// For pseudo files (e.g., those on sockfs), this device is also "pseudo".
    ///
    /// Corresponds to `st_dev`.
    pub container_dev_id: DeviceId,

    /// The device ID of the inode itself, if this inode represents a
    /// special device file (character or block).
    ///
    /// Corresponds to `st_rdev`.
    pub self_dev_id: Option<DeviceId>,
}

impl Metadata {
    pub fn new_dir(ino: u64, mode: InodeMode, blk_size: usize) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 2,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 1,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::Dir,
            mode,
            nr_hard_links: 2,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id: DeviceId::null(),
            self_dev_id: None,
        }
    }

    pub fn new_file(ino: u64, mode: InodeMode, blk_size: usize) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 0,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::File,
            mode,
            nr_hard_links: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id: DeviceId::null(),
            self_dev_id: None,
        }
    }

    pub fn new_symlink(ino: u64, mode: InodeMode, blk_size: usize) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 0,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::SymLink,
            mode,
            nr_hard_links: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id: DeviceId::null(),
            self_dev_id: None,
        }
    }

    pub fn new_device(ino: u64, mode: InodeMode, blk_size: usize, device: &dyn Device) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 0,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::from(device.type_()),
            mode,
            nr_hard_links: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id: DeviceId::null(),
            self_dev_id: Some(device.id()),
        }
    }
}

pub enum MknodType {
    NamedPipe,
    CharDevice(u64),
    BlockDevice(u64),
}

impl MknodType {
    pub fn device_type(&self) -> Option<DeviceType> {
        match self {
            MknodType::NamedPipe => None,
            MknodType::CharDevice(_) => Some(DeviceType::Char),
            MknodType::BlockDevice(_) => Some(DeviceType::Block),
        }
    }
}

/// I/O operations in an [`Inode`].
///
/// This abstracts the common I/O operations used by both [`Inode`] (for regular files) and
/// [`FileIo`] (for special files).
pub trait InodeIo {
    /// Reads data from the file into the given `VmWriter`.
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize>;

    /// Writes data from the given `VmReader` into the file.
    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize>;
}

pub trait Inode: Any + InodeIo + Send + Sync {
    fn size(&self) -> usize;

    fn resize(&self, new_size: usize) -> Result<()>;

    fn metadata(&self) -> Metadata;

    fn ino(&self) -> u64;

    fn type_(&self) -> InodeType;

    fn mode(&self) -> Result<InodeMode>;

    fn set_mode(&self, mode: InodeMode) -> Result<()>;

    fn owner(&self) -> Result<Uid>;

    fn set_owner(&self, uid: Uid) -> Result<()>;

    fn group(&self) -> Result<Gid>;

    fn set_group(&self, gid: Gid) -> Result<()>;

    fn atime(&self) -> Duration;

    fn set_atime(&self, time: Duration);

    fn mtime(&self) -> Duration;

    fn set_mtime(&self, time: Duration);

    fn ctime(&self) -> Duration;

    fn set_ctime(&self, time: Duration);

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        None
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        None
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn unlink(&self, name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_link(&self, target: &str) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn sync_all(&self) -> Result<()> {
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        Ok(())
    }

    /// Manipulates a range of space of the file according to the specified allocate mode,
    /// the manipulated range starts at `offset` and continues for `len` bytes.
    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        return_errno!(Errno::EOPNOTSUPP);
    }

    fn fs(&self) -> Arc<dyn FileSystem>;

    /// Returns whether a VFS dentry for this inode should be put into the dentry cache.
    ///
    /// The dentry cache in the VFS layer can accelerate the lookup of inodes. So usually,
    /// it is preferable to use the dentry cache. And thus, the default return value of this method
    /// is `true`.
    ///
    /// But this caching can raise consistency issues in certain use cases. Specifically, the dentry
    /// cache works on the assumption that all FS operations go through the dentry layer first.
    /// This is why the dentry cache can reflect the up-to-date FS state. Yet, this assumption
    /// may be broken. If the inodes of a file system may "disappear" without unlinking through the
    /// VFS layer, then their dentries should not be cached. For example, an inode in procfs
    /// (say, `/proc/1/fd/2`) can "disappear" without notice from the perspective of the dentry cache.
    /// So for such inodes, they are incompatible with the dentry cache. And this method returns `false`.
    ///
    /// Note that if any ancestor directory of an inode has this method returns `false`, then
    /// this inode would not be cached by the dentry cache, even when the method of this
    /// inode returns `true`.
    fn is_dentry_cacheable(&self) -> bool {
        true
    }

    /// Returns the end position for [`SeekFrom::End`].
    ///
    /// [`SeekFrom::End`]: super::SeekFrom::End
    fn seek_end(&self) -> Option<usize> {
        if self.type_() == InodeType::File {
            Some(self.size())
        } else {
            // This depends on the file system. For example, seeking directories from the end
            // succeeds under procfs and btrfs but fails under tmpfs. Here, we just choose a
            // safe default to reject it.
            // TODO: Carefully check the Linux behavior of each file system and adjust ours
            // accordingly.
            None
        }
    }

    /// Gets the extension of this inode.
    fn extension(&self) -> &Extension;

    fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        Err(Error::new(Errno::EOPNOTSUPP))
    }

    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        Err(Error::new(Errno::EOPNOTSUPP))
    }

    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        Err(Error::new(Errno::EOPNOTSUPP))
    }

    fn remove_xattr(&self, name: XattrName) -> Result<()> {
        Err(Error::new(Errno::EOPNOTSUPP))
    }

    /// Used to check for read/write/execute permissions on a file.
    ///
    /// Similar to Linux, using "fsuid" here allows setting filesystem permissions
    /// without changing the "normal" uids for other tasks.
    fn check_permission(&self, mut perm: Permission) -> Result<()> {
        let creds = match Task::current() {
            Some(task) => match task.as_posix_thread() {
                Some(thread) => thread.credentials(),
                None => return Ok(()),
            },
            None => return Ok(()),
        };

        // With DAC_OVERRIDE capability, the user can bypass some permission checks.
        if creds.effective_capset().contains(CapSet::DAC_OVERRIDE) {
            // Read/write DACs are always overridable.
            perm -= Permission::MAY_READ | Permission::MAY_WRITE;

            // Executable DACs are overridable when there is at least one exec bit set.
            if perm.may_exec() {
                let metadata = self.metadata();
                let mode = metadata.mode;

                if mode.is_owner_executable()
                    || mode.is_group_executable()
                    || mode.is_other_executable()
                {
                    perm -= Permission::MAY_EXEC;
                } else {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "root execute permission denied: no execute bits set"
                    );
                }
            }
        }

        perm =
            perm.intersection(Permission::MAY_READ | Permission::MAY_WRITE | Permission::MAY_EXEC);
        let metadata = self.metadata();
        let mode = metadata.mode;

        if metadata.uid == creds.fsuid() {
            if (perm.may_read() && !mode.is_owner_readable())
                || (perm.may_write() && !mode.is_owner_writable())
                || (perm.may_exec() && !mode.is_owner_executable())
            {
                return_errno_with_message!(Errno::EACCES, "owner permission check failed");
            }
        } else if metadata.gid == creds.fsgid() {
            if (perm.may_read() && !mode.is_group_readable())
                || (perm.may_write() && !mode.is_group_writable())
                || (perm.may_exec() && !mode.is_group_executable())
            {
                return_errno_with_message!(Errno::EACCES, "group permission check failed");
            }
        } else if (perm.may_read() && !mode.is_other_readable())
            || (perm.may_write() && !mode.is_other_writable())
            || (perm.may_exec() && !mode.is_other_executable())
        {
            return_errno_with_message!(Errno::EACCES, "other permission check failed");
        }

        Ok(())
    }
}

impl dyn Inode {
    pub fn downcast_ref<T: Inode>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    pub fn writer(&self, from_offset: usize) -> InodeWriter<'_> {
        InodeWriter {
            inner: self,
            offset: from_offset,
        }
    }

    pub fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_at(offset, &mut writer, StatusFlags::empty())
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_at(offset, &mut reader, StatusFlags::empty())
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn read_bytes_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_at(offset, &mut writer, StatusFlags::O_DIRECT)
    }

    #[cfg_attr(not(ktest), expect(dead_code))]
    pub fn write_bytes_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_at(offset, &mut reader, StatusFlags::O_DIRECT)
    }
}

pub struct InodeWriter<'a> {
    inner: &'a dyn Inode,
    offset: usize,
}

impl Write for InodeWriter<'_> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        let write_len = self
            .inner
            .write_at(self.offset, &mut reader, StatusFlags::empty())
            .map_err(|_| IoError::new(IoErrorKind::WriteZero, "failed to write buffer"))?;
        self.offset += write_len;
        Ok(write_len)
    }

    #[inline]
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}

impl Debug for dyn Inode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Inode")
            .field("metadata", &self.metadata())
            .field("fs", &self.fs())
            .finish()
    }
}

/// An extension is a set of object groups that is attached to an inode.
///
/// In this structure, we do not specify the exact type, but instead use [`Any`], which makes the
/// FS types (e.g., [`Inode`]) independent of the kernel types. This allows the file system
/// implementation to exist outside the kernel.
#[derive(Debug)]
pub struct Extension {
    group1: Once<ThinBox<dyn Any + Send + Sync>>,
    group2: Once<ThinBox<dyn Any + Send + Sync>>,
}

impl Extension {
    /// Creates a new, empty extension.
    pub fn new() -> Self {
        Self {
            group1: Once::new(),
            group2: Once::new(),
        }
    }

    /// Gets the first extension group.
    pub fn group1(&self) -> &Once<ThinBox<dyn Any + Send + Sync>> {
        &self.group1
    }

    /// Gets the second extension group.
    pub fn group2(&self) -> &Once<ThinBox<dyn Any + Send + Sync>> {
        &self.group2
    }
}

/// A symbolic link.
#[derive(Debug, Clone)]
pub enum SymbolicLink {
    /// A plain text.
    ///
    /// This is the most common type of symbolic link.
    /// Symbolic links on a normal FS are of this variant.
    Plain(String),
    /// An file object residing at a FS path.
    ///
    /// This variant is intended to support the special ProcFS symbolic links,
    /// such as `/proc/[pid]/fd/[fd]` and `/proc/[pid]/exe`.
    Path(Path),
}
