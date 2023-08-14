use crate::fs::device::Device;
use crate::fs::utils::{
    DirentVisitor, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
};
use crate::prelude::*;
use crate::vm::vmo::Vmo;

use block_io::bid::BlockId;
use core::time::Duration;
use ext2::{Ext2Inode, FilePerm, FileType};
use jinux_frame::vm::VmFrame;
use jinux_rights::Full;

impl Inode for Ext2Inode {
    fn len(&self) -> usize {
        self.file_size() as _
    }

    fn resize(&self, new_size: usize) {
        self.resize(new_size).unwrap()
    }

    fn blocks_len(&self) -> usize {
        self.blocks_count() as usize * self.fs().super_block().block_size()
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

    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = BlockId::new(idx as u32);
        Ok(self.read_block(bid, frame)?)
    }

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = BlockId::new(idx as u32);
        Ok(self.write_block(bid, frame)?)
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        Ok(self.read_at(offset, buf)?)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        Ok(self.write_at(offset, buf)?)
    }

    fn create_with_pages(
        &self,
        name: &str,
        type_: InodeType,
        mode: InodeMode,
        pages: &Vmo<Full>,
    ) -> Result<Arc<dyn Inode>> {
        Ok(self.create(name, type_.into(), mode.into(), pages)?)
    }

    fn mknod_with_pages(
        &self,
        name: &str,
        mode: InodeMode,
        dev: Arc<dyn Device>,
        pages: &Vmo<Full>,
    ) -> Result<Arc<dyn Inode>> {
        todo!();
    }

    fn readdir_at_with_pages(
        &self,
        offset: usize,
        visitor: &mut dyn DirentVisitor,
        pages: &Vmo<Full>,
    ) -> Result<usize> {
        todo!();
    }

    fn link_with_pages(&self, old: &Arc<dyn Inode>, name: &str, pages: &Vmo<Full>) -> Result<()> {
        todo!();
    }

    fn unlink_with_pages(&self, name: &str, pages: &Vmo<Full>) -> Result<()> {
        todo!();
    }

    fn rmdir_with_pages(&self, name: &str, pages: &Vmo<Full>) -> Result<()> {
        todo!();
    }

    fn lookup_with_pages(&self, name: &str, pages: &Vmo<Full>) -> Result<Arc<dyn Inode>> {
        Ok(self.lookup(name, pages)?)
    }

    fn rename_with_pages(
        &self,
        old_name: &str,
        dir_pages: &Vmo<Full>,
        target: &Arc<dyn Inode>,
        new_name: &str,
        target_pages: &Vmo<Full>,
    ) -> Result<()> {
        todo!();
    }

    fn read_link_with_pages(&self, pages: &Vmo<Full>) -> Result<String> {
        Ok(self.read_link(pages)?)
    }

    fn write_link_with_pages(&self, target: &str, pages: &Vmo<Full>) -> Result<()> {
        Ok(self.write_link(target, pages)?)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        todo!();
    }

    fn sync(&self) -> Result<()> {
        Ok(self.sync_metadata()?)
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
