// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use aster_block::BlockDevice;
use device_id::DeviceId;
use hadris_io::Error as HadrisIoError;
use hadris_iso::sync::{
    directory::DirectoryRef,
    io::LogicalSector,
    read::{DirEntry, IsoImage},
};

use super::block_io::BlockDeviceIo;
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags, mkmod},
        utils::{DirentVisitor, NAME_MAX},
        vfs::{
            file_system::{FileSystem, FsEventSubscriberStats, SuperBlock},
            inode::{Extension, FileOps, Inode, Metadata, SymbolicLink},
            registry::{FsCreationCtx, FsProperties, FsType},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

type IsoInner = IsoImage<BlockDeviceIo>;

static ISO9660_TYPE: Iso9660Type = Iso9660Type { name: "iso9660" };
static ISOFS_TYPE: Iso9660Type = Iso9660Type { name: "isofs" };

pub(super) fn init() {
    crate::fs::vfs::registry::register(&ISO9660_TYPE).unwrap();
    crate::fs::vfs::registry::register(&ISOFS_TYPE).unwrap();
}

#[derive(Debug)]
struct Iso9660FileSystem {
    inner: IsoInner,
    root: Arc<Iso9660Inode>,
    block_device: Arc<dyn BlockDevice>,
    fs_event_subscriber_stats: FsEventSubscriberStats,
}

impl Iso9660FileSystem {
    fn open(block_device: Arc<dyn BlockDevice>) -> Result<Arc<Self>> {
        let inner =
            IsoInner::open(BlockDeviceIo::new(block_device.clone())).map_err(map_hadris_io_err)?;
        let root_dir = inner.root_dir().dir_ref();
        Ok(Arc::new_cyclic(|weak_self| Self {
            inner,
            root: Arc::new(Iso9660Inode::new(
                weak_self.clone(),
                Vec::new(),
                Iso9660InodeKind::Dir(root_dir),
            )),
            block_device,
            fs_event_subscriber_stats: FsEventSubscriberStats::new(),
        }))
    }

    fn open_dir(&self, dir_ref: DirectoryRef) -> hadris_iso::sync::read::IsoDir<'_, BlockDeviceIo> {
        self.inner.open_dir(dir_ref)
    }

    fn container_device_id(&self) -> DeviceId {
        self.block_device.id()
    }
}

impl FileSystem for Iso9660FileSystem {
    fn name(&self) -> &'static str {
        "iso9660"
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn root_inode(&self) -> Arc<dyn Inode> {
        self.root.clone()
    }

    fn sb(&self) -> SuperBlock {
        SuperBlock::new(0x9660, 2048, NAME_MAX, self.container_device_id())
    }

    fn fs_event_subscriber_stats(&self) -> &FsEventSubscriberStats {
        &self.fs_event_subscriber_stats
    }
}

#[derive(Clone, Debug)]
enum Iso9660InodeKind {
    Dir(DirectoryRef),
    File(Box<DirEntry>),
    Symlink(String),
}

impl Iso9660InodeKind {
    fn inode_type(&self) -> InodeType {
        match self {
            Self::Dir(_) => InodeType::Dir,
            Self::File(_) => InodeType::File,
            Self::Symlink(_) => InodeType::SymLink,
        }
    }

    fn size(&self) -> usize {
        match self {
            Self::Dir(dir_ref) => dir_ref.size,
            Self::File(entry) => usize::try_from(entry.total_size()).unwrap_or(usize::MAX),
            Self::Symlink(target) => target.len(),
        }
    }

    fn mode(&self) -> InodeMode {
        match self {
            Self::Dir(_) => mkmod!(a+rx, u+w),
            Self::File(entry) => mode_from_entry(entry).unwrap_or_else(|| mkmod!(a+r)),
            Self::Symlink(_) => mkmod!(a+rwx),
        }
    }
}

#[derive(Debug)]
struct Iso9660Inode {
    fs: Weak<Iso9660FileSystem>,
    path: Vec<String>,
    kind: Iso9660InodeKind,
    extension: Extension,
}

impl Iso9660Inode {
    fn new(fs: Weak<Iso9660FileSystem>, path: Vec<String>, kind: Iso9660InodeKind) -> Self {
        Self {
            fs,
            path,
            kind,
            extension: Extension::new(),
        }
    }

    fn fs(&self) -> Arc<Iso9660FileSystem> {
        self.fs.upgrade().expect("ISO inode must have a live fs")
    }

    fn child_path(&self, name: &str) -> Vec<String> {
        child_path_from(&self.path, name)
    }

    fn read_file_range(
        &self,
        entry: &DirEntry,
        offset: usize,
        writer: &mut VmWriter,
    ) -> Result<usize> {
        let total_size = usize::try_from(entry.total_size())
            .map_err(|_| Error::with_message(Errno::EOVERFLOW, "ISO file is too large"))?;
        if offset >= total_size {
            return Ok(0);
        }

        let read_len = writer.avail().min(total_size - offset);
        let mut buf = vec![0; read_len];
        let mut logical_start = 0usize;
        let mut copied = 0usize;

        for extent in entry.extents() {
            let extent_len = extent.length as usize;
            let logical_end = logical_start
                .checked_add(extent_len)
                .ok_or_else(|| Error::with_message(Errno::EOVERFLOW, "ISO extent overflow"))?;
            let wanted_start = offset.max(logical_start);
            let wanted_end = (offset + read_len).min(logical_end);
            if wanted_start < wanted_end {
                let extent_offset = wanted_start - logical_start;
                let dst_offset = wanted_start - offset;
                let len = wanted_end - wanted_start;
                let byte_offset = extent
                    .sector
                    .0
                    .checked_mul(2048)
                    .and_then(|base| base.checked_add(extent_offset))
                    .ok_or_else(|| {
                        Error::with_message(Errno::EOVERFLOW, "ISO byte offset overflow")
                    })?;
                self.fs()
                    .inner
                    .read_bytes_at(byte_offset as u64, &mut buf[dst_offset..dst_offset + len])
                    .map_err(map_hadris_io_err)?;
                copied = copied.max(dst_offset + len);
            }
            logical_start = logical_end;
            if logical_start >= offset + read_len {
                break;
            }
        }

        let mut reader = VmReader::from(&buf[..copied]).to_fallible();
        writer.write_fallible(&mut reader).map_err(|(err, _)| err)?;
        Ok(copied)
    }
}

impl FileOps for Iso9660Inode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        match &self.kind {
            Iso9660InodeKind::File(entry) => self.read_file_range(entry, offset, writer),
            Iso9660InodeKind::Symlink(target) => {
                if offset >= target.len() {
                    return Ok(0);
                }
                let read_len = writer.avail().min(target.len() - offset);
                let mut reader =
                    VmReader::from(&target.as_bytes()[offset..offset + read_len]).to_fallible();
                writer.write_fallible(&mut reader).map_err(|(err, _)| err)?;
                Ok(read_len)
            }
            Iso9660InodeKind::Dir(_) => {
                return_errno_with_message!(Errno::EISDIR, "cannot read an ISO directory as a file")
            }
        }
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let Iso9660InodeKind::Dir(dir_ref) = self.kind else {
            return_errno_with_message!(Errno::ENOTDIR, "cannot read entries from an ISO file");
        };

        let fs = self.fs();
        let dir = fs.open_dir(dir_ref);
        let mut index = 0usize;
        let mut visited = 0usize;
        let self_ino = path_ino(&self.path);
        let parent_ino = parent_ino(&self.path);

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

        for entry in dir.entries() {
            let entry = entry.map_err(map_hadris_io_err)?;
            if entry.is_special() {
                continue;
            }
            let name = display_name(&entry);
            if index >= offset {
                let child_path = self.child_path(&name);
                visitor.visit(
                    &name,
                    path_ino(&child_path),
                    inode_kind_from_entry(&fs, &entry)?.inode_type(),
                    index + 1,
                )?;
                visited += 1;
            }
            index += 1;
        }

        Ok(visited)
    }
}

impl Inode for Iso9660Inode {
    fn size(&self) -> usize {
        self.kind.size()
    }

    fn resize(&self, _new_size: usize) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            ino: self.ino(),
            size: self.size(),
            optimal_block_size: 2048,
            nr_sectors_allocated: self.size().div_ceil(512),
            last_access_at: Duration::ZERO,
            last_modify_at: Duration::ZERO,
            last_meta_change_at: Duration::ZERO,
            type_: self.kind.inode_type(),
            mode: self.kind.mode(),
            nr_hard_links: if matches!(self.kind, Iso9660InodeKind::Dir(_)) {
                2
            } else {
                1
            },
            uid: Uid::new_root(),
            gid: Gid::new_root(),
            container_dev_id: self.fs().container_device_id(),
            self_dev_id: None,
            birth_at: Duration::ZERO,
        }
    }

    fn ino(&self) -> u64 {
        path_ino(&self.path)
    }

    fn type_(&self) -> InodeType {
        self.kind.inode_type()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.kind.mode())
    }

    fn set_mode(&self, _mode: InodeMode) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn owner(&self) -> Result<Uid> {
        Ok(Uid::new_root())
    }

    fn set_owner(&self, _uid: Uid) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn group(&self) -> Result<Gid> {
        Ok(Gid::new_root())
    }

    fn set_group(&self, _gid: Gid) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
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

    fn create(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn rmdir(&self, _name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        if name == "." {
            return Ok(Arc::new(Self::new(
                self.fs.clone(),
                self.path.clone(),
                self.kind.clone(),
            )));
        }
        if name == ".." {
            let mut path = self.path.clone();
            let _ = path.pop();
            let root_ref = self.fs().inner.root_dir().dir_ref();
            return Ok(Arc::new(Self::new(
                self.fs.clone(),
                path,
                Iso9660InodeKind::Dir(root_ref),
            )));
        }

        let Iso9660InodeKind::Dir(dir_ref) = self.kind else {
            return_errno_with_message!(Errno::ENOTDIR, "ISO inode is not a directory");
        };

        let fs = self.fs();
        let dir = fs.open_dir(dir_ref);
        for entry in dir.entries() {
            let entry = entry.map_err(map_hadris_io_err)?;
            if entry.is_special() {
                continue;
            }
            let entry_name = display_name(&entry);
            if entry_name == name {
                return Ok(Arc::new(Self::new(
                    self.fs.clone(),
                    self.child_path(name),
                    inode_kind_from_entry(&fs, &entry)?,
                )));
            }
        }

        return_errno_with_message!(Errno::ENOENT, "ISO entry not found")
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        match &self.kind {
            Iso9660InodeKind::Symlink(target) => Ok(SymbolicLink::Plain(target.clone())),
            _ => return_errno_with_message!(Errno::EINVAL, "ISO inode is not a symbolic link"),
        }
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        return_errno_with_message!(Errno::EROFS, "ISO 9660 is read-only")
    }

    fn sync_all(&self) -> Result<()> {
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs()
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }
}

struct Iso9660Type {
    name: &'static str,
}

impl FsType for Iso9660Type {
    fn name(&self) -> &'static str {
        self.name
    }

    fn properties(&self) -> FsProperties {
        FsProperties::NEED_DISK
    }

    fn create(&self, fs_creation_ctx: &FsCreationCtx) -> Result<Arc<dyn FileSystem>> {
        let block_device = fs_creation_ctx.resolve_block_device()?;
        Iso9660FileSystem::open(block_device).map(|fs| fs as Arc<dyn FileSystem>)
    }

    fn sysnode(&self) -> Option<Arc<dyn aster_systree::SysNode>> {
        None
    }
}

fn inode_kind_from_entry(fs: &Iso9660FileSystem, entry: &DirEntry) -> Result<Iso9660InodeKind> {
    if let Some(rrip) = &entry.rrip
        && let Some(target) = &rrip.symlink_target
    {
        return Ok(Iso9660InodeKind::Symlink(target.clone()));
    }

    if entry.is_directory() {
        let dir_ref = entry.as_dir_ref(&fs.inner).map_err(map_hadris_io_err)?;
        Ok(Iso9660InodeKind::Dir(dir_ref))
    } else {
        Ok(Iso9660InodeKind::File(Box::new(entry.clone())))
    }
}

fn mode_from_entry(entry: &DirEntry) -> Option<InodeMode> {
    let rrip = entry.rrip.as_ref()?;
    let attrs = rrip.posix_attributes?;
    let mode = attrs.file_mode.read() as u16;
    Some(InodeMode::from_bits_truncate(mode & 0o7777))
}

fn display_name(entry: &DirEntry) -> String {
    let mut name = entry.display_name().into_owned();
    if let Some((base, version)) = name.rsplit_once(';')
        && version.as_bytes().iter().all(u8::is_ascii_digit)
    {
        name = base.to_string();
    }
    name
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

fn map_hadris_io_err(_err: HadrisIoError) -> Error {
    Error::with_message(Errno::EIO, "ISO 9660 filesystem operation failed")
}

#[expect(dead_code)]
fn dir_ref_from_extent(extent: usize, size: usize) -> DirectoryRef {
    DirectoryRef {
        extent: LogicalSector(extent),
        size,
    }
}
