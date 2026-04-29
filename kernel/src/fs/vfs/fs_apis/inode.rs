// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use alloc::boxed::ThinBox;
use core::time::Duration;

use device_id::DeviceId;
use no_std_io2::io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult, Write};
use ostd::task::Task;
use spin::Once;

use super::{
    file_system::FileSystem,
    xattr::{XattrName, XattrNamespace, XattrSetFlags},
};
use crate::{
    device::{Device, DeviceType},
    fs::{
        file::{AccessMode, FileIo, InodeMode, InodeType, Permission, StatusFlags},
        utils::DirentVisitor,
        vfs::path::Path,
    },
    prelude::*,
    process::{Gid, Uid, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    time::clocks::RealTimeCoarseClock,
    vm::vmo::Vmo,
};

#[derive(Clone, Copy, Debug)]
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
    /// For persisted files, this device could be an on-disk partition
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
    pub fn new_dir(ino: u64, mode: InodeMode, blk_size: usize, container_dev_id: DeviceId) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            ino,
            size: 2,
            optimal_block_size: blk_size,
            nr_sectors_allocated: 0,
            last_access_at: now,
            last_modify_at: now,
            last_meta_change_at: now,
            type_: InodeType::Dir,
            mode,
            nr_hard_links: 2,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id,
            self_dev_id: None,
        }
    }

    pub fn new_file(
        ino: u64,
        mode: InodeMode,
        blk_size: usize,
        container_dev_id: DeviceId,
    ) -> Self {
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
            container_dev_id,
            self_dev_id: None,
        }
    }

    pub fn new_symlink(
        ino: u64,
        mode: InodeMode,
        blk_size: usize,
        container_dev_id: DeviceId,
    ) -> Self {
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
            container_dev_id,
            self_dev_id: None,
        }
    }

    pub fn new_device(
        ino: u64,
        mode: InodeMode,
        blk_size: usize,
        device: &dyn Device,
        container_dev_id: DeviceId,
    ) -> Self {
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
            container_dev_id,
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

bitflags! {
    /// Policy that controls when the VFS revalidates cached inode entries.
    ///
    /// # Why revalidation is needed
    ///
    /// Normally, inode creation and deletion go through VFS operations such as
    /// `create`, `unlink`, `mkdir`, and `rmdir`. This lets the VFS keep its
    /// dentry cache consistent with the underlying filesystem.
    ///
    /// However, some filesystems create or delete inodes without going through
    /// the VFS. For example, procfs entries under `/proc` appear and disappear
    /// as processes are forked and reaped. Networked filesystems may also be
    /// modified by remote peers. For such filesystems, the VFS must revalidate
    /// cached entries by asking the parent directory whether a cached child is
    /// still valid.
    ///
    /// # Revalidation protocol
    ///
    /// A directory inode declares its policy through
    /// [`Inode::revalidation_policy`].
    ///
    /// - [`REVALIDATE_EXISTS`](Self::REVALIDATE_EXISTS) makes the VFS call
    ///   [`Inode::revalidate_exists`] on positive cache hits. If it returns
    ///   `false`, the cached entry is dropped and the lookup is retried.
    /// - [`REVALIDATE_ABSENT`](Self::REVALIDATE_ABSENT) makes the VFS call
    ///   [`Inode::revalidate_absent`] on negative cache hits. If it returns
    ///   `false`, the cached entry is dropped and the lookup is retried.
    ///
    /// If neither flag is set, cached entries are trusted unconditionally. The
    /// policy must be fixed for each inode. Non-directory inodes should return
    /// an empty policy.
    ///
    /// # Performance
    ///
    /// The revalidation callbacks run on every matching cache hit, so
    /// filesystems should keep them cheap.
    pub struct RevalidationPolicy: u8 {
        /// Revalidate positive cache entries (cached child inodes).
        ///
        /// Set this when the directory may spontaneously
        /// _lose_ children without going through VFS `unlink`/`rmdir`.
        /// For example, procfs sets this on `/proc`
        /// because PID directories disappear when processes exit.
        ///
        /// When set, every positive cache hit calls
        /// [`Inode::revalidate_exists`] on the parent directory.
        const REVALIDATE_EXISTS = 1 << 0;

        /// Revalidate negative cache entries (names known to be absent).
        ///
        /// Set this when the directory may spontaneously _gain_ children
        /// without going through VFS `create`/`mkdir`/`link`.
        /// For example, procfs sets this on `/proc`
        /// because PID directories appear when processes are forked.
        ///
        /// When set, every negative cache hit calls
        /// [`Inode::revalidate_absent`] on the parent directory.
        const REVALIDATE_ABSENT = 1 << 1;
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

    /// Returns the revalidation policy for cached children of this directory.
    ///
    /// See [`RevalidationPolicy`] for the full protocol description.
    ///
    /// Default: empty (no revalidation).
    /// Correct for filesystems where all mutations go through the VFS.
    fn revalidation_policy(&self) -> RevalidationPolicy {
        RevalidationPolicy::empty()
    }

    /// Checks whether a cached child inode still exists under this directory.
    ///
    /// Called on a positive cache hit
    /// when [`RevalidationPolicy::REVALIDATE_EXISTS`] is set.
    /// Returns `true` if `child` is still a valid child named `name`;
    /// `false` if the entry should be dropped and the lookup retried.
    ///
    /// See [`RevalidationPolicy`] for the full protocol description.
    ///
    /// # Precondition
    ///
    /// Only called when `self.revalidation_policy()` includes `REVALIDATE_EXISTS`.
    /// Otherwise, the return value is considered garbage.
    fn revalidate_exists(&self, _name: &str, _child: &dyn Inode) -> bool {
        true
    }

    /// Checks whether a name is still absent under this directory.
    ///
    /// Called on a negative cache hit
    /// when [`RevalidationPolicy::REVALIDATE_ABSENT`] is set.
    /// Returns `true` if `name` is still absent;
    /// `false` if a child may now exist and the lookup should be retried.
    ///
    /// See [`RevalidationPolicy`] for the full protocol description.
    ///
    /// # Precondition
    ///
    /// Only called when `self.revalidation_policy()` includes `REVALIDATE_ABSENT`.
    /// Otherwise, the return value is considered garbage.
    fn revalidate_absent(&self, _name: &str) -> bool {
        true
    }

    /// Returns the end position for [`SeekFrom::End`].
    ///
    /// [`SeekFrom::End`]: crate::fs::file::SeekFrom::End
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
#[derive(Clone, Debug)]
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

/// Represents the various operation modes for fallocate.
///
/// Each mode determines whether the target disk space within a file
/// will be allocated, deallocated, or zeroed, among other operations.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum FallocMode {
    /// Allocates disk space within the range specified.
    Allocate,
    /// Like `Allocate`, but does not change the file size.
    AllocateKeepSize,
    /// Makes shared file data extents private to guarantee subsequent writes.
    AllocateUnshareRange,
    /// Deallocates space (creates a hole) while keeping the file size unchanged.
    PunchHoleKeepSize,
    /// Converts a file range to zeros, expanding the file if necessary.
    ZeroRange,
    /// Like `ZeroRange`, but does not change the file size.
    ZeroRangeKeepSize,
    /// Removes a range of bytes without leaving a hole.
    CollapseRange,
    /// Inserts space within a file without overwriting existing data.
    InsertRange,
}
