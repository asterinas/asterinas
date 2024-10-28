// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use core::time::Duration;

use aster_rights::Full;

use crate::{
    fs::{
        ext2::{FilePerm, Inode as Ext2Inode},
        utils::{
            DirentVisitor, Extension, FallocMode, FileSystem, Inode, InodeMode, InodeType,
            IoctlCmd, Metadata, MknodType,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::vmo::Vmo,
};

impl Inode for Ext2Inode {
    fn size(&self) -> usize {
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
            type_: self.inode_type(),
            mode: InodeMode::from(self.file_perm()),
            nlinks: self.hard_links() as _,
            uid: Uid::new(self.uid()),
            gid: Gid::new(self.gid()),
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

    fn ctime(&self) -> Duration {
        self.ctime()
    }

    fn set_ctime(&self, time: Duration) {
        self.set_ctime(time)
    }

    fn ino(&self) -> u64 {
        self.ino() as _
    }

    fn type_(&self) -> InodeType {
        self.inode_type()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(InodeMode::from(self.file_perm()))
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.set_file_perm(mode.into());
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(Uid::new(self.uid()))
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.set_uid(uid.into());
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(Gid::new(self.gid()))
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.set_gid(gid.into());
        Ok(())
    }

    fn page_cache(&self) -> Option<Vmo<Full>> {
        Some(self.page_cache())
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.read_at(offset, writer)
    }

    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.read_direct_at(offset, writer)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.write_at(offset, reader)
    }

    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.write_direct_at(offset, reader)
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Ok(self.create(name, type_, mode.into())?)
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        let inode_type = type_.inode_type();
        let inode = match type_ {
            MknodType::CharDeviceNode(dev) | MknodType::BlockDeviceNode(dev) => {
                let inode = self.create(name, inode_type, mode.into())?;
                inode.set_device_id(dev.id().into()).unwrap();
                inode
            }
            _ => todo!(),
        };

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

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.fallocate(mode, offset, len)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        Err(Error::new(Errno::EINVAL))
    }

    fn sync_all(&self) -> Result<()> {
        self.sync_all()?;
        self.fs().block_device().sync()?;
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        self.sync_data()?;
        self.fs().block_device().sync()?;
        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs()
    }

    fn extension(&self) -> Option<&Extension> {
        Some(self.extension())
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
