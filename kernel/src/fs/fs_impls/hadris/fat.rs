// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_block::{BlockDevice, bio::BioStatus};
use device_id::DeviceId;
use hadris_fat::{
    FatError,
    raw::DirEntryAttrFlags,
    read::FileReader,
    sync::{
        DirectoryEntry, FatDir, FatFs as HadrisFatFs, FatFsWriteExt, FileEntry, write::FileWriter,
    },
};

use super::block_io::BlockDeviceIo;
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags, mkmod},
        utils::{DirentVisitor, NAME_MAX},
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::{Extension, FileOps, Inode, Metadata, MknodType, SymbolicLink},
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::PageCache,
};

type FatInner = HadrisFatFs<BlockDeviceIo>;
type FatFileWriter<'a> = FileWriter<'a, BlockDeviceIo>;

const ZERO_FILL_CHUNK_SIZE: usize = 4096;

static FAT_TYPE: FatType = FatType { name: "fat" };
static VFAT_TYPE: FatType = FatType { name: "vfat" };
static MSDOS_TYPE: FatType = FatType { name: "msdos" };

pub(super) fn init() {
    crate::fs::vfs::registry::register(&FAT_TYPE).unwrap();
    crate::fs::vfs::registry::register(&VFAT_TYPE).unwrap();
    crate::fs::vfs::registry::register(&MSDOS_TYPE).unwrap();
}

#[derive(Debug)]
struct FatFileSystem {
    root: Arc<FatInode>,
    block_device: Arc<dyn BlockDevice>,
    operation_lock: Mutex<()>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl FatFileSystem {
    fn open(block_device: Arc<dyn BlockDevice>) -> Result<Arc<Self>> {
        FatInner::open(BlockDeviceIo::new(block_device.clone())).map_err(map_fat_err)?;
        Ok(Arc::new_cyclic(|weak_self| Self {
            root: Arc::new(FatInode::new(
                weak_self.clone(),
                Vec::new(),
                FatInodeKind::Dir,
            )),
            block_device,
            operation_lock: Mutex::new(()),
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        }))
    }

    fn open_inner(&self) -> Result<FatInner> {
        FatInner::open(BlockDeviceIo::new(self.block_device.clone())).map_err(map_fat_err)
    }

    fn open_dir<'a>(inner: &'a FatInner, path: &[String]) -> Result<FatDir<'a, BlockDeviceIo>> {
        let mut dir = inner.root_dir();
        for component in path {
            dir = dir.open_dir(component).map_err(map_fat_err)?;
        }
        Ok(dir)
    }

    fn find_entry_in(inner: &FatInner, path: &[String]) -> Result<FileEntry> {
        let (parent_path, name) = split_path(path)?;
        let parent = Self::open_dir(inner, parent_path)?;
        parent
            .find(name)
            .map_err(map_fat_err)?
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "FAT entry not found"))
    }

    fn find_entry(&self, path: &[String]) -> Result<FileEntry> {
        let _lock = self.operation_lock.lock();
        let inner = self.open_inner()?;
        Self::find_entry_in(&inner, path)
    }

    fn read_file(&self, path: &[String]) -> Result<Vec<u8>> {
        let _lock = self.operation_lock.lock();
        let inner = self.open_inner()?;
        let (parent_path, name) = split_path(path)?;
        let parent = Self::open_dir(&inner, parent_path)?;
        let entry = parent
            .find(name)
            .map_err(map_fat_err)?
            .ok_or_else(|| Error::with_message(Errno::ENOENT, "FAT entry not found"))?;
        let mut reader = FileReader::new(&inner, &entry).map_err(map_fat_err)?;
        let mut buf = vec![0; entry.size()];
        let mut offset = 0;

        while offset < buf.len() {
            let read_len = reader.read(&mut buf[offset..]).map_err(map_fat_err)?;
            if read_len == 0 {
                break;
            }
            offset += read_len;
        }
        buf.truncate(offset);
        Ok(buf)
    }

    fn rewrite_file(&self, path: &[String], contents: &[u8]) -> Result<()> {
        let _lock = self.operation_lock.lock();
        let inner = self.open_inner()?;
        let entry = Self::find_entry_in(&inner, path)?;
        inner.truncate(&entry, 0).map_err(map_fat_err)?;

        let entry = Self::find_entry_in(&inner, path)?;
        let mut writer = inner.write_file(&entry).map_err(map_fat_err)?;
        write_all_to_fat(&mut writer, contents)?;
        writer.finish().map_err(map_fat_err)
    }

    fn append_to_file(&self, path: &[String], offset: usize, contents: &[u8]) -> Result<()> {
        let _lock = self.operation_lock.lock();
        let inner = self.open_inner()?;
        let entry = Self::find_entry_in(&inner, path)?;
        let current_size = entry.size();
        if offset < current_size {
            return_errno_with_message!(Errno::EINVAL, "FAT append offset is before file end");
        }

        let mut writer = FatFileWriter::new_append(&inner, &entry).map_err(map_fat_err)?;
        write_zeroes_to_fat(&mut writer, offset - current_size)?;
        write_all_to_fat(&mut writer, contents)?;
        writer.finish().map_err(map_fat_err)
    }

    fn resize_file(&self, path: &[String], new_size: usize) -> Result<()> {
        let _lock = self.operation_lock.lock();
        let inner = self.open_inner()?;
        let entry = Self::find_entry_in(&inner, path)?;
        let current_size = entry.size();
        match new_size.cmp(&current_size) {
            core::cmp::Ordering::Less => inner.truncate(&entry, new_size).map_err(map_fat_err),
            core::cmp::Ordering::Equal => Ok(()),
            core::cmp::Ordering::Greater => {
                let mut writer = FatFileWriter::new_append(&inner, &entry).map_err(map_fat_err)?;
                write_zeroes_to_fat(&mut writer, new_size - current_size)?;
                writer.finish().map_err(map_fat_err)
            }
        }
    }

    fn container_device_id(&self) -> DeviceId {
        self.block_device.id()
    }
}

impl FileSystem for FatFileSystem {
    fn name(&self) -> &'static str {
        "vfat"
    }

    fn sync(&self) -> Result<()> {
        if self.block_device.sync()? != BioStatus::Complete {
            return_errno_with_message!(Errno::EIO, "failed to flush FAT block device");
        }
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        SuperBlock::new(0x4d44, 512, NAME_MAX, self.container_device_id())
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum FatInodeKind {
    Dir,
    File,
}

impl FatInodeKind {
    fn from_entry(entry: &FileEntry) -> Self {
        if entry.is_directory() {
            Self::Dir
        } else {
            Self::File
        }
    }

    fn inode_type(self) -> InodeType {
        match self {
            Self::Dir => InodeType::Dir,
            Self::File => InodeType::File,
        }
    }

    fn mode(self) -> InodeMode {
        match self {
            Self::Dir => mkmod!(a+rwx),
            Self::File => mkmod!(a+rw),
        }
    }
}

#[derive(Debug)]
struct FatInode {
    fs: Weak<FatFileSystem>,
    path: RwLock<Vec<String>>,
    kind: RwLock<FatInodeKind>,
    extension: Extension,
}

impl FatInode {
    fn new(fs: Weak<FatFileSystem>, path: Vec<String>, kind: FatInodeKind) -> Self {
        Self {
            fs,
            path: RwLock::new(path),
            kind: RwLock::new(kind),
            extension: Extension::new(),
        }
    }

    fn fs(&self) -> Arc<FatFileSystem> {
        self.fs.upgrade().expect("FAT inode must have a live fs")
    }

    fn path(&self) -> Vec<String> {
        self.path.read().clone()
    }

    fn child_path(&self, name: &str) -> Vec<String> {
        let mut path = self.path();
        path.push(name.to_string());
        path
    }

    fn kind(&self) -> FatInodeKind {
        *self.kind.read()
    }

    fn refresh_kind(&self) -> FatInodeKind {
        if self.path.read().is_empty() {
            return FatInodeKind::Dir;
        }

        let kind = self
            .fs()
            .find_entry(&self.path())
            .map(|entry| FatInodeKind::from_entry(&entry))
            .unwrap_or_else(|_| self.kind());
        *self.kind.write() = kind;
        kind
    }

    fn read_file(&self) -> Result<Vec<u8>> {
        if self.refresh_kind() != FatInodeKind::File {
            return_errno_with_message!(Errno::EISDIR, "cannot read a FAT directory as a file");
        }
        self.fs().read_file(&self.path())
    }
}

impl FileOps for FatInode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        let contents = self.read_file()?;
        if offset >= contents.len() {
            return Ok(0);
        }

        let read_len = writer.avail().min(contents.len() - offset);
        let mut reader = VmReader::from(&contents[offset..offset + read_len]).to_fallible();
        writer.write_fallible(&mut reader).map_err(|(err, _)| err)?;
        Ok(read_len)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        if self.refresh_kind() != FatInodeKind::File {
            return_errno_with_message!(Errno::EISDIR, "cannot write a FAT directory");
        }

        let write_len = reader.remain();
        let write_end = offset
            .checked_add(write_len)
            .ok_or_else(|| Error::with_message(Errno::EFBIG, "FAT write offset overflow"))?;
        let mut incoming = vec![0; write_len];
        let mut writer = VmWriter::from(incoming.as_mut_slice()).to_fallible();
        reader.read_fallible(&mut writer).map_err(|(err, _)| err)?;

        let fs = self.fs();
        let path = self.path();

        let current_size = fs.find_entry(&path)?.size();
        if offset >= current_size {
            fs.append_to_file(&path, offset, &incoming)?;
        } else {
            let mut contents = fs.read_file(&path)?;
            if contents.len() < write_end {
                contents.resize(write_end, 0);
            }
            contents[offset..write_end].copy_from_slice(&incoming);
            fs.rewrite_file(&path, &contents)?;
        }
        Ok(write_len)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        if self.refresh_kind() != FatInodeKind::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "cannot read entries from a FAT file");
        }

        let fs = self.fs();
        let path = self.path();
        let _lock = fs.operation_lock.lock();
        let inner = fs.open_inner()?;
        let dir = FatFileSystem::open_dir(&inner, &path)?;
        let mut index = 0usize;
        let mut visited = 0usize;
        let self_ino = path_ino(&path);
        let parent_ino = parent_ino(&path);

        if index >= offset {
            visitor.visit(".", self_ino, InodeType::Dir, index + 1)?;
            visited += 1;
        }
        index += 1;

        if index >= offset {
            visitor.visit("..", parent_ino, InodeType::Dir, index + 1)?;
            visited += 1;
        }
        index += 1;

        let mut iter = dir.entries();
        while let Some(entry) = iter.next_entry() {
            let DirectoryEntry::Entry(entry) = entry.map_err(map_fat_err)?;
            if entry.attributes().contains(DirEntryAttrFlags::VOLUME_ID) {
                continue;
            }
            if index >= offset {
                let name = entry.name();
                let child_path = child_path_from(&path, name.as_ref());
                visitor.visit(
                    name.as_ref(),
                    path_ino(&child_path),
                    FatInodeKind::from_entry(&entry).inode_type(),
                    index + 1,
                )?;
                visited += 1;
            }
            index += 1;
        }

        Ok(visited)
    }
}

impl Inode for FatInode {
    fn size(&self) -> usize {
        if self.refresh_kind() == FatInodeKind::Dir {
            return 0;
        }
        self.fs()
            .find_entry(&self.path())
            .map(|entry| entry.size())
            .unwrap_or(0)
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        if self.refresh_kind() != FatInodeKind::File {
            return_errno_with_message!(Errno::EISDIR, "cannot resize a FAT directory");
        }

        self.fs().resize_file(&self.path(), new_size)
    }

    fn metadata(&self) -> Metadata {
        let kind = self.refresh_kind();
        Metadata {
            ino: self.ino(),
            size: self.size(),
            optimal_block_size: 512,
            nr_sectors_allocated: self.size().div_ceil(512),
            last_access_at: Duration::ZERO,
            last_modify_at: Duration::ZERO,
            last_meta_change_at: Duration::ZERO,
            type_: kind.inode_type(),
            mode: kind.mode(),
            nr_hard_links: if kind == FatInodeKind::Dir { 2 } else { 1 },
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id: self.fs().container_device_id(),
            self_dev_id: None,
            birth_at: Duration::ZERO,
        }
    }

    fn ino(&self) -> u64 {
        path_ino(&self.path())
    }

    fn type_(&self) -> InodeType {
        self.refresh_kind().inode_type()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.refresh_kind().mode())
    }

    fn set_mode(&self, _mode: InodeMode) -> Result<()> {
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(Uid::new_root())
    }

    fn set_owner(&self, _uid: Uid) -> Result<()> {
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(Gid::new_root())
    }

    fn set_group(&self, _gid: Gid) -> Result<()> {
        Ok(())
    }

    fn atime(&self) -> Duration {
        Duration::ZERO
    }

    fn set_atime(&self, _time: Duration) {}

    fn mtime(&self) -> Duration {
        Duration::ZERO
    }

    fn set_mtime(&self, _time: Duration) {}

    fn ctime(&self) -> Duration {
        Duration::ZERO
    }

    fn set_ctime(&self, _time: Duration) {}

    fn page_cache(&self) -> Option<PageCache> {
        None
    }

    fn create(&self, name: &str, type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        if self.refresh_kind() != FatInodeKind::Dir {
            return_errno_with_message!(Errno::ENOTDIR, "FAT parent is not a directory");
        }

        let fs = self.fs();
        let _lock = fs.operation_lock.lock();
        let inner = fs.open_inner()?;
        let parent = FatFileSystem::open_dir(&inner, &self.path())?;
        let child_path = self.child_path(name);
        let kind = match type_ {
            InodeType::Dir => {
                inner.create_dir(&parent, name).map_err(map_fat_err)?;
                FatInodeKind::Dir
            }
            InodeType::File => {
                inner.create_file(&parent, name).map_err(map_fat_err)?;
                FatInodeKind::File
            }
            _ => return_errno_with_message!(Errno::EOPNOTSUPP, "FAT supports only files and dirs"),
        };

        Ok(Arc::new(FatInode::new(self.fs.clone(), child_path, kind)))
    }

    fn mknod(&self, _name: &str, _mode: InodeMode, _type_: MknodType) -> Result<Arc<dyn Inode>> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "FAT does not support special files")
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "FAT does not support hard links")
    }

    fn unlink(&self, name: &str) -> Result<()> {
        let fs = self.fs();
        let _lock = fs.operation_lock.lock();
        let inner = fs.open_inner()?;
        let path = self.child_path(name);
        let entry = FatFileSystem::find_entry_in(&inner, &path)?;
        if entry.is_directory() {
            return_errno_with_message!(Errno::EISDIR, "cannot unlink a FAT directory");
        }
        inner.delete(&entry).map_err(map_fat_err)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        let fs = self.fs();
        let _lock = fs.operation_lock.lock();
        let inner = fs.open_inner()?;
        let path = self.child_path(name);
        let entry = FatFileSystem::find_entry_in(&inner, &path)?;
        if !entry.is_directory() {
            return_errno_with_message!(Errno::ENOTDIR, "FAT entry is not a directory");
        }
        inner.delete(&entry).map_err(map_fat_err)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if name == "." {
            return Ok(Arc::new(FatInode::new(
                self.fs.clone(),
                self.path(),
                FatInodeKind::Dir,
            )));
        }
        if name == ".." {
            let mut path = self.path();
            let _ = path.pop();
            return Ok(Arc::new(FatInode::new(
                self.fs.clone(),
                path,
                FatInodeKind::Dir,
            )));
        }

        let path = self.child_path(name);
        let entry = self.fs().find_entry(&path)?;
        Ok(Arc::new(FatInode::new(
            self.fs.clone(),
            path,
            FatInodeKind::from_entry(&entry),
        )))
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        let target = target
            .downcast_ref::<FatInode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "target is not a FAT inode"))?;
        let source_fs = self.fs();
        let target_fs = target.fs();
        if !Arc::ptr_eq(&source_fs, &target_fs) {
            return_errno_with_message!(Errno::EXDEV, "cannot rename across filesystems");
        }

        let source_path = self.child_path(old_name);
        let dest_path = target.child_path(new_name);
        let _lock = source_fs.operation_lock.lock();
        let inner = source_fs.open_inner()?;
        if let Ok(existing) = FatFileSystem::find_entry_in(&inner, &dest_path) {
            inner.delete(&existing).map_err(map_fat_err)?;
        }

        let entry = FatFileSystem::find_entry_in(&inner, &source_path)?;
        let dest_dir = FatFileSystem::open_dir(&inner, &target.path())?;
        inner
            .rename(&entry, &dest_dir, new_name)
            .map_err(map_fat_err)
            .map(|_| ())
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        return_errno_with_message!(Errno::EINVAL, "FAT does not support symbolic links")
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        return_errno_with_message!(Errno::EOPNOTSUPP, "FAT does not support symbolic links")
    }

    fn sync_all(&self) -> Result<()> {
        self.fs().sync()
    }

    fn sync_data(&self) -> Result<()> {
        self.fs().sync()
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs()
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

struct FatType {
    name: &'static str,
}

impl FsType for FatType {
    fn name(&self) -> &'static str {
        self.name
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let block_device = fs_creation_ctx.resolve_block_device()?;
        FatFileSystem::open(block_device).map(|fs| fs as Arc<dyn FileSystem>)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

fn split_path(path: &[String]) -> Result<(&[String], &str)> {
    let (name, parent) = path
        .split_last()
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "FAT root has no file entry"))?;
    Ok((parent, name.as_str()))
}

fn child_path_from(path: &[String], name: &str) -> Vec<String> {
    let mut child = path.to_vec();
    child.push(name.to_string());
    child
}

fn parent_ino(path: &[String]) -> u64 {
    if path.is_empty() {
        return 1;
    }
    path_ino(&path[..path.len() - 1])
}

fn path_ino(path: &[String]) -> u64 {
    if path.is_empty() {
        return 1;
    }

    let mut hash = 0xcbf29ce484222325u64;
    for component in path {
        for byte in component.as_bytes() {
            hash ^= *byte as u64;
            hash = hash.wrapping_mul(0x100000001b3);
        }
        hash ^= b'/' as u64;
        hash = hash.wrapping_mul(0x100000001b3);
    }

    hash.max(2)
}

fn write_all_to_fat(writer: &mut FatFileWriter<'_>, contents: &[u8]) -> Result<()> {
    let mut offset = 0;
    while offset < contents.len() {
        let write_len = writer.write(&contents[offset..]).map_err(map_fat_err)?;
        if write_len == 0 {
            return_errno_with_message!(Errno::ENOSPC, "FAT write made no progress");
        }
        offset += write_len;
    }
    Ok(())
}

fn write_zeroes_to_fat(writer: &mut FatFileWriter<'_>, len: usize) -> Result<()> {
    let zeroes = [0; ZERO_FILL_CHUNK_SIZE];
    let mut remaining = len;
    while remaining > 0 {
        let chunk_len = remaining.min(zeroes.len());
        write_all_to_fat(writer, &zeroes[..chunk_len])?;
        remaining -= chunk_len;
    }
    Ok(())
}

fn map_fat_err(err: FatError) -> Error {
    match err {
        FatError::EntryNotFound => Error::new(Errno::ENOENT),
        FatError::AlreadyExists => Error::new(Errno::EEXIST),
        FatError::NotADirectory => Error::new(Errno::ENOTDIR),
        FatError::NotAFile => Error::new(Errno::EISDIR),
        FatError::NoFreeSpace | FatError::DirectoryFull => Error::new(Errno::ENOSPC),
        FatError::DirectoryNotEmpty => Error::new(Errno::ENOTEMPTY),
        FatError::InvalidFilename | FatError::InvalidShortFilename | FatError::InvalidPath => {
            Error::new(Errno::EINVAL)
        }
        _ => Error::with_message(Errno::EIO, "FAT filesystem operation failed"),
    }
}
