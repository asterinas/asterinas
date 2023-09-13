use crate::fs::device::Device;
use crate::fs::utils::{
    DirentVisitor, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata, PageCache,
};
use crate::prelude::*;

use core::time::Duration;
use ext2::{Ext2Inode, FilePerm, FileType};

impl Inode for Ext2Inode {
    fn len(&self) -> usize {
        self.file_size() as _
    }

    fn resize(&self, new_size: usize) {
        self.resize(new_size).unwrap()
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
            rdev: 0, // TODO: device ID for device inode,
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

    fn set_mode(&self, mode: InodeMode) {
        self.set_file_perm(mode.into());
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        Ok(self.read_at(offset, buf)?)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        Ok(self.read_direct_at(offset, buf)?)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        Ok(self.write_at(offset, buf)?)
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        Ok(self.write_direct_at(offset, buf)?)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Ok(self.create::<PageCache>(name, type_.into(), mode.into())?)
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: Arc<dyn Device>) -> Result<Arc<dyn Inode>> {
        todo!();
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        todo!();
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        todo!();
    }

    fn unlink(&self, name: &str) -> Result<()> {
        todo!();
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        todo!();
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Ok(self.lookup::<PageCache>(name)?)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        todo!();
    }

    fn read_link(&self) -> Result<String> {
        Ok(self.read_link()?)
    }

    fn write_link(&self, target: &str) -> Result<()> {
        Ok(self.write_link(target)?)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        todo!();
    }

    fn sync(&self) -> Result<()> {
        self.sync_data()?;
        self.sync_metadata()?;
        Ok(())
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
