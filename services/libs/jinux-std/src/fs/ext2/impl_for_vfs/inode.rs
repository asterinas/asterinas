use crate::fs::device::Device;
use crate::fs::utils::{
    DirentVisitor, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
};
use crate::prelude::*;
use crate::vm::vmo::Vmo;

use block_io::bid::{BlockId, BLOCK_SIZE};
use block_io::bio::BioBuf;
use core::time::Duration;
use jinux_frame::vm::VmFrame;
use jinux_frame::GenericIo;
use jinux_rights::Full;

use crate::fs::ext2::{Ext2Inode, FilePerm, FileType};

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

    fn set_mode(&self, mode: InodeMode) {
        self.set_file_perm(mode.into());
    }

    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = BlockId::new(idx as u32);

        // Note: the as_slice method of VmFrame is unsafe
        let mut block_buf = vec![0u8; BLOCK_SIZE];
        let mut bio_buf = BioBuf::from_slice_mut(&mut block_buf);
        self.read_block(bid, &mut bio_buf)?;
        frame.write_bytes(0, bio_buf.as_slice())?;

        Ok(())
    }

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        let bid = BlockId::new(idx as u32);

        // Note: the as_slice method of VmFrame is unsafe
        let mut block_buf = vec![0u8; BLOCK_SIZE];
        frame.read_bytes(0, &mut block_buf)?;
        let bio_buf = BioBuf::from_slice(&block_buf);
        self.write_block(bid, &bio_buf)?;

        Ok(())
    }

    fn page_cache(&self) -> Option<Vmo<Full>> {
        Some(self.page_cache().pages().dup().unwrap())
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
        Ok(self.create(name, type_.into(), mode.into())?)
    }

    fn mknod(&self, name: &str, mode: InodeMode, dev: Arc<dyn Device>) -> Result<Arc<dyn Inode>> {
        let inode = self.create(name, InodeType::from(dev.type_()).into(), mode.into())?;
        inode.write_device_id(dev.id().into()).unwrap();
        Ok(inode)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Ok(self.lookup(name)?)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let try_readdir = |offset: &mut usize, visitor: &mut dyn DirentVisitor| -> Result<()> {
            let dir_entry_reader = self.dir_entry_reader(*offset)?;
            for (entry_offset, dir_entry) in dir_entry_reader {
                visitor.visit(
                    dir_entry.name(),
                    dir_entry.ino() as u64,
                    InodeType::from(dir_entry.type_()),
                    dir_entry.record_len(),
                )?;
                *offset = entry_offset
            }

            Ok(())
        };

        let mut iterate_offset = offset;
        match try_readdir(&mut iterate_offset, visitor) {
            Err(e) if iterate_offset == offset => Err(e),
            _ => Ok(iterate_offset - offset),
        }
    }

    fn link(&self, old: &Arc<dyn Inode>, name: &str) -> Result<()> {
        let old = old
            .downcast_ref::<Ext2Inode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "not same fs"))?;
        Ok(self.link(old, name)?)
    }

    fn unlink(&self, name: &str) -> Result<()> {
        Ok(self.unlink(name)?)
    }

    fn rmdir(&self, name: &str) -> Result<()> {
        Ok(self.rmdir(name)?)
    }

    fn rename(&self, old_name: &str, target: &Arc<dyn Inode>, new_name: &str) -> Result<()> {
        let target = target
            .downcast_ref::<Ext2Inode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "not same fs"))?;
        Ok(self.rename(old_name, target, new_name)?)
    }

    fn read_link(&self) -> Result<String> {
        Ok(self.read_link()?)
    }

    fn write_link(&self, target: &str) -> Result<()> {
        Ok(self.write_link(target)?)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        Err(Error::new(Errno::EINVAL))
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
