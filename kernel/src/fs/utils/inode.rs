// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::{any::TypeId, time::Duration};

use aster_rights::Full;
use core2::io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult, Write};

use super::{DirentVisitor, FallocMode, FileSystem, IoctlCmd};
use crate::{
    events::IoEvents,
    fs::device::{Device, DeviceType},
    prelude::*,
    process::{signal::PollHandle, Gid, Uid},
    time::clocks::RealTimeCoarseClock,
    vm::vmo::Vmo,
};

#[repr(u16)]
#[derive(Copy, Clone, Debug, Eq, PartialEq, TryFromInt)]
pub enum InodeType {
    NamedPipe = 0o010000,
    CharDevice = 0o020000,
    Dir = 0o040000,
    BlockDevice = 0o060000,
    File = 0o100000,
    SymLink = 0o120000,
    Socket = 0o140000,
}

impl InodeType {
    pub fn support_read(&self) -> bool {
        matches!(
            self,
            InodeType::File | InodeType::Socket | InodeType::CharDevice | InodeType::BlockDevice
        )
    }

    pub fn support_write(&self) -> bool {
        matches!(
            self,
            InodeType::File | InodeType::Socket | InodeType::CharDevice | InodeType::BlockDevice
        )
    }

    pub fn is_regular_file(&self) -> bool {
        *self == InodeType::File
    }

    pub fn is_directory(&self) -> bool {
        *self == InodeType::Dir
    }

    pub fn is_device(&self) -> bool {
        *self == InodeType::BlockDevice || *self == InodeType::CharDevice
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
}

impl From<DeviceType> for InodeType {
    fn from(type_: DeviceType) -> InodeType {
        match type_ {
            DeviceType::CharDevice => InodeType::CharDevice,
            DeviceType::BlockDevice => InodeType::BlockDevice,
            DeviceType::MiscDevice => InodeType::CharDevice,
        }
    }
}

bitflags! {
    pub struct InodeMode: u16 {
        /// set-user-ID
        const S_ISUID = 0o4000;
        /// set-group-ID
        const S_ISGID = 0o2000;
        /// sticky bit
        const S_ISVTX = 0o1000;
        /// read by owner
        const S_IRUSR = 0o0400;
        /// write by owner
        const S_IWUSR = 0o0200;
        /// execute/search by owner
        const S_IXUSR = 0o0100;
        /// read by group
        const S_IRGRP = 0o0040;
        /// write by group
        const S_IWGRP = 0o0020;
        /// execute/search by group
        const S_IXGRP = 0o0010;
        /// read by others
        const S_IROTH = 0o0004;
        /// write by others
        const S_IWOTH = 0o0002;
        /// execute/search by others
        const S_IXOTH = 0o0001;
    }
}

impl InodeMode {
    pub fn is_readable(&self) -> bool {
        self.contains(Self::S_IRUSR)
    }

    pub fn is_writable(&self) -> bool {
        self.contains(Self::S_IWUSR)
    }

    pub fn is_executable(&self) -> bool {
        self.contains(Self::S_IXUSR)
    }

    pub fn has_sticky_bit(&self) -> bool {
        self.contains(Self::S_ISVTX)
    }

    pub fn has_set_uid(&self) -> bool {
        self.contains(Self::S_ISUID)
    }

    pub fn has_set_gid(&self) -> bool {
        self.contains(Self::S_ISGID)
    }
}

#[derive(Debug, Clone, Copy)]
pub struct Metadata {
    pub dev: u64,
    pub ino: u64,
    pub size: usize,
    pub blk_size: usize,
    pub blocks: usize,
    pub atime: Duration,
    pub mtime: Duration,
    pub ctime: Duration,
    pub type_: InodeType,
    pub mode: InodeMode,
    pub nlinks: usize,
    pub uid: Uid,
    pub gid: Gid,
    pub rdev: u64,
}

impl Metadata {
    pub fn new_dir(ino: u64, mode: InodeMode, blk_size: usize) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            dev: 0,
            ino,
            size: 2,
            blk_size,
            blocks: 1,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::Dir,
            mode,
            nlinks: 2,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }

    pub fn new_file(ino: u64, mode: InodeMode, blk_size: usize) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::File,
            mode,
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }

    pub fn new_symlink(ino: u64, mode: InodeMode, blk_size: usize) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::SymLink,
            mode,
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }
    pub fn new_device(ino: u64, mode: InodeMode, blk_size: usize, device: &dyn Device) -> Self {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::from(device.type_()),
            mode,
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: device.id().into(),
        }
    }

    pub fn new_socket(ino: u64, mode: InodeMode, blk_size: usize) -> Metadata {
        let now = RealTimeCoarseClock::get().read_time();
        Self {
            dev: 0,
            ino,
            size: 0,
            blk_size,
            blocks: 0,
            atime: now,
            mtime: now,
            ctime: now,
            type_: InodeType::Socket,
            mode,
            nlinks: 1,
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            rdev: 0,
        }
    }
}

pub enum MknodType {
    NamedPipeNode,
    CharDeviceNode(Arc<dyn Device>),
    BlockDeviceNode(Arc<dyn Device>),
}

impl MknodType {
    pub fn inode_type(&self) -> InodeType {
        match self {
            MknodType::NamedPipeNode => InodeType::NamedPipe,
            MknodType::CharDeviceNode(_) => InodeType::CharDevice,
            MknodType::BlockDeviceNode(_) => InodeType::BlockDevice,
        }
    }
}

impl From<Arc<dyn Device>> for MknodType {
    fn from(device: Arc<dyn Device>) -> Self {
        let inode_type: InodeType = device.type_().into();
        match inode_type {
            InodeType::CharDevice => Self::CharDeviceNode(device),
            InodeType::BlockDevice => Self::BlockDeviceNode(device),
            _ => unreachable!(),
        }
    }
}

pub trait Inode: Any + Sync + Send {
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

    fn page_cache(&self) -> Option<Vmo<Full>> {
        None
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        Err(Error::new(Errno::EISDIR))
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
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

    fn read_link(&self) -> Result<String> {
        Err(Error::new(Errno::EISDIR))
    }

    fn write_link(&self, target: &str) -> Result<()> {
        Err(Error::new(Errno::EISDIR))
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
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

    fn poll(&self, mask: IoEvents, _poller: Option<&mut PollHandle>) -> IoEvents {
        let events = IoEvents::IN | IoEvents::OUT;
        events & mask
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

    fn is_seekable(&self) -> bool {
        true
    }

    /// Get the extension of this inode
    fn extension(&self) -> Option<&Extension> {
        None
    }
}

impl dyn Inode {
    pub fn downcast_ref<T: Inode>(&self) -> Option<&T> {
        (self as &dyn Any).downcast_ref::<T>()
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        if !self.type_().support_read() {
            return_errno!(Errno::EISDIR);
        }

        let file_size = self.size();
        if buf.len() < file_size {
            buf.resize(file_size, 0);
        }

        let mut writer = VmWriter::from(&mut buf[..file_size]).to_fallible();
        self.read_at(0, &mut writer)
    }

    pub fn read_direct_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        if !self.type_().support_read() {
            return_errno!(Errno::EISDIR);
        }

        let file_size = self.size();
        if buf.len() < file_size {
            buf.resize(file_size, 0);
        }

        let mut writer = VmWriter::from(&mut buf[..file_size]).to_fallible();
        self.read_direct_at(0, &mut writer)
    }

    pub fn writer(&self, from_offset: usize) -> InodeWriter {
        InodeWriter {
            inner: self,
            offset: from_offset,
        }
    }

    pub fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_at(offset, &mut writer)
    }

    pub fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_at(offset, &mut reader)
    }

    pub fn read_bytes_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let mut writer = VmWriter::from(buf).to_fallible();
        self.read_direct_at(offset, &mut writer)
    }

    pub fn write_bytes_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let mut reader = VmReader::from(buf).to_fallible();
        self.write_direct_at(offset, &mut reader)
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
            .write_at(self.offset, &mut reader)
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

/// An extension is a set of objects that is attached to
/// an inode.
///
/// Each objects of an extension is of different types.
/// In other words, types are used as the keys to get and
/// set the objects in an extension.
#[derive(Debug)]
pub struct Extension {
    data: RwLock<BTreeMap<TypeId, Arc<dyn Any + Send + Sync>>>,
}

impl Extension {
    pub fn new() -> Self {
        Self {
            data: RwLock::new(BTreeMap::new()),
        }
    }

    /// Get an object of `Arc<T>`.
    pub fn get<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let read_guard = self.data.read();
        read_guard
            .get(&TypeId::of::<T>())
            .and_then(|arc_any| Arc::downcast::<T>(arc_any.clone()).ok())
    }

    /// Try to get an object of `Arc<T>`. If no object of the type exists,
    /// put the default value for the type, then return it.
    pub fn get_or_put_default<T: Any + Send + Sync + Default>(&self) -> Arc<T> {
        let mut write_guard = self.data.write();
        let type_id = TypeId::of::<T>();
        let arc_any = write_guard.entry(type_id).or_insert_with(|| {
            let obj = T::default();
            Arc::new(obj) as Arc<dyn Any + Send + Sync>
        });
        Arc::downcast::<T>(arc_any.clone()).unwrap()
    }

    /// Put an object of `Arc<T>`. If there exists one object of the type,
    /// then the old one is returned.
    pub fn put<T: Any + Send + Sync>(&self, obj: Arc<T>) -> Option<Arc<T>> {
        let mut write_guard = self.data.write();
        write_guard
            .insert(TypeId::of::<T>(), obj as Arc<dyn Any + Send + Sync>)
            .and_then(|arc_any| Arc::downcast::<T>(arc_any).ok())
    }

    /// Delete an object of `Arc<T>`. If there exists one object of the type,
    /// then the old one is returned.
    pub fn del<T: Any + Send + Sync>(&self) -> Option<Arc<T>> {
        let mut write_guard = self.data.write();
        write_guard
            .remove(&TypeId::of::<T>())
            .and_then(|arc_any| Arc::downcast::<T>(arc_any).ok())
    }
}

impl Clone for Extension {
    fn clone(&self) -> Self {
        Self {
            data: RwLock::new(self.data.read().clone()),
        }
    }
}

impl Default for Extension {
    fn default() -> Self {
        Self::new()
    }
}
