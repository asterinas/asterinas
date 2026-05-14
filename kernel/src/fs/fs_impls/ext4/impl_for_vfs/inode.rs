// SPDX-License-Identifier: MPL-2.0

//! Inode adapter for Ext4.

use core::time::Duration;

use device_id::DeviceId;

use crate::{
    fs::{
        ext4::{FilePerm, Inode as Ext4Inode},
        file::{AccessMode, FileIo, InodeMode, InodeType, StatusFlags},
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{Extension, FallocMode, Inode, InodeIo, Metadata, MknodType, SymbolicLink},
            xattr::{XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::vmo::Vmo,
};

impl InodeIo for Ext4Inode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        self.read_at(offset, writer)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        return_errno!(Errno::EROFS)
    }
}

impl Inode for Ext4Inode {
    fn size(&self) -> usize {
        self.file_size()
    }

    fn resize(&self, _new_size: usize) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn metadata(&self) -> Metadata {
        let fs = self.fs();
        Metadata {
            ino: self.ino() as u64,
            size: self.file_size(),
            optimal_block_size: fs.block_size(),
            nr_sectors_allocated: 0,
            last_access_at: Duration::ZERO,
            last_modify_at: Duration::ZERO,
            last_meta_change_at: Duration::ZERO,
            type_: self.inode_type(),
            mode: InodeMode::from_bits_truncate(self.file_perm().bits() as u32),
            nr_hard_links: 0,
            uid: Uid::new(0),
            gid: Gid::new(0),
            container_dev_id: fs.container_device_id(),
            self_dev_id: None,
        }
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

    fn ino(&self) -> u64 {
        self.ino() as u64
    }

    fn type_(&self) -> InodeType {
        self.inode_type()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(InodeMode::from_bits_truncate(self.file_perm().bits() as u32))
    }

    fn set_mode(&self, _mode: InodeMode) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn owner(&self) -> Result<Uid> {
        Ok(Uid::new(0))
    }

    fn set_owner(&self, _uid: Uid) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn group(&self) -> Result<Gid> {
        Ok(Gid::new(0))
    }

    fn set_group(&self, _gid: Gid) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        Some(self.page_cache())
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        None
    }

    fn create(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        return_errno!(Errno::EROFS)
    }

    fn mknod(&self, _name: &str, _mode: InodeMode, _type_: MknodType) -> Result<Arc<dyn Inode>> {
        return_errno!(Errno::EROFS)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Ok(self.lookup(name)?)
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.readdir_at(offset, visitor)
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn rmdir(&self, _name: &str) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        return_errno!(Errno::EROFS)
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        return_errno!(Errno::EROFS)
    }

    fn fallocate(&self, _mode: FallocMode, _offset: usize, _len: usize) -> Result<()> {
        return_errno!(Errno::EROFS)
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
        // TODO: implement extension
        unimplemented!("ext4 inode extension")
    }

    fn set_xattr(
        &self,
        _name: XattrName,
        _value_reader: &mut VmReader,
        _flags: XattrSetFlags,
    ) -> Result<()> {
        return_errno!(Errno::EOPNOTSUPP)
    }

    fn get_xattr(&self, _name: XattrName, _value_writer: &mut VmWriter) -> Result<usize> {
        return_errno!(Errno::ENODATA)
    }

    fn list_xattr(&self, _namespace: XattrNamespace, _list_writer: &mut VmWriter) -> Result<usize> {
        Ok(0)
    }

    fn remove_xattr(&self, _name: XattrName) -> Result<()> {
        return_errno!(Errno::EOPNOTSUPP)
    }
}
