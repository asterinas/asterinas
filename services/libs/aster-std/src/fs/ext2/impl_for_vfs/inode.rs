use crate::fs::device::Device;
use crate::fs::ext2::{FilePerm, FileType, Inode as Ext2Inode};
use crate::fs::utils::{
    DirentVisitor, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
};
use crate::prelude::*;
use crate::vm::vmo::Vmo;

use aster_rights::Full;
use core::time::Duration;

impl Inode for Ext2Inode {
    fn len(&self) -> usize {
        self.file_size() as _
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.resize(new_size)
    }

    fn metadata(&self) -> Metadata {
        Metadata {
            dev: 0, // TODO: ID of block device
            ino: self.ino() as _,
            size: self.file_size() as _,
            blk_size: self.fs().super_block().block_size(),
            blocks: self.blocks_count() as _,
            atime: self.atime(),
            mtime: self.mtime(),
            ctime: self.ctime(),
            type_: InodeType::from(self.file_type()),
            mode: InodeMode::from(self.file_perm()),
            nlinks: self.hard_links() as _,
            uid: self.uid() as _,
            gid: self.gid() as _,
            rdev: self.device_id(),
        }
    }

    fn atime(&self) -> Duration {
        self.atime()
    }

    fn set_atime(&self, time: Duration) {
        self.set_atime(time)
    }

    fn mtime(&self) -> Duration {
        self.mtime()
    }

    fn set_mtime(&self, time: Duration) {
        self.set_mtime(time)
    }

    fn ino(&self) -> u64 {
        self.ino() as _
    }

    fn type_(&self) -> InodeType {
        InodeType::from(self.file_type())
    }

    fn mode(&self) -> InodeMode {
        InodeMode::from(self.file_perm())
    }

    fn set_mode(&self, mode: InodeMode) {
        self.set_file_perm(mode.into());
    }

    fn page_cache(&self) -> Option<Vmo<Full>> {
        Some(self.page_cache())
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.read_at(offset, buf)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.read_direct_at(offset, buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.write_at(offset, buf)
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.write_direct_at(offset, buf)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Ok(self.create(name, type_.into(), mode.into())?)
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: Arc<dyn Device>) -> Result<Arc<dyn Inode>> {
        let inode = self.create(name, InodeType::from(dev.type_()).into(), mode.into())?;
        inode.set_device_id(dev.id().into()).unwrap();
        Ok(inode)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Ok(self.lookup(name)?)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.readdir_at(offset, visitor)
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        let old = old
            .downcast_ref::<Ext2Inode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "not same fs"))?;
        self.link(old, name)
    }

    fn unlink(&self, name: &str) -> Result<()> {
        self.unlink(name)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        self.rmdir(name)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        let target = target
            .downcast_ref::<Ext2Inode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "not same fs"))?;
        self.rename(old_name, target, new_name)
    }

    fn read_link(&self) -> Result<String> {
        self.read_link()
    }

    fn write_link(&self, target: &str) -> Result<()> {
        self.write_link(target)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        Err(Error::new(Errno::EINVAL))
    }

    fn sync(&self) -> Result<()> {
        self.sync_all()
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs()
    }
}

impl From<FilePerm> for InodeMode {
    fn from(perm: FilePerm) -> Self {
        Self::from_bits_truncate(perm.bits() as _)
    }
}

impl From<InodeMode> for FilePerm {
    fn from(mode: InodeMode) -> Self {
        Self::from_bits_truncate(mode.bits() as _)
    }
}

impl From<FileType> for InodeType {
    fn from(type_: FileType) -> Self {
        Self::try_from(type_ as u32).unwrap()
    }
}

impl From<InodeType> for FileType {
    fn from(type_: InodeType) -> Self {
        Self::try_from(type_ as u16).unwrap()
    }
}
