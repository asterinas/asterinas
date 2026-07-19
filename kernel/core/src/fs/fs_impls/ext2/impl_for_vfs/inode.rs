// SPDX-License-Identifier: MPL-2.0

//! VFS inode trait implementations for the ext2 [`Inode`](super::super::Inode).
//!
//! Wires the ext2 `Inode` into the VFS layer by implementing the `InodeIo`
//! and `Inode` traits. The implementation converts VFS requests into
//! ext2-internal operations, including symlink results and the inode's VFS
//! extension slot.

use core::time::Duration;

use aster_block::bio::BioStatus;
use device_id::DeviceId;

use crate::{
    device,
    fs::{
        file::{AccessMode, InodeMode, InodeType, PerOpenFileOps, Permission, StatusFlags},
        fs_impls::ext2::{FilePerm, Inode as Ext2Inode},
        utils::DirentVisitor,
        vfs::{
            file_system::FileSystem,
            inode::{
                Extension, FallocMode, FileOps, Inode, Metadata, MknodType, RenameMode,
                SymbolicLink,
            },
            xattr::{XattrName, XattrNamespace, XattrSetFlags},
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::page_cache::PageCache,
};

impl FileOps for Ext2Inode {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if status_flags.contains(StatusFlags::O_DIRECT) {
            self.read_direct_at(offset, writer)
        } else {
            self.read_at(offset, writer)
        }
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        if status_flags.contains(StatusFlags::O_DIRECT) {
            self.write_direct_at(offset, reader)
        } else {
            self.write_at(offset, reader)
        }
    }

    fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.readdir_at(offset, visitor)
    }
}

impl Inode for Ext2Inode {
    fn size(&self) -> usize {
        self.file_size()
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.resize(new_size)
    }

    fn metadata(&self) -> Metadata {
        self.metadata()
    }

    fn ino(&self) -> u64 {
        self.ino() as u64
    }

    fn type_(&self) -> InodeType {
        self.inode_type()
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.mode())
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.set_mode(mode)
    }

    fn owner(&self) -> Result<Uid> {
        Ok(Uid::new(self.uid()))
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.set_uid(uid.into())
    }

    fn group(&self) -> Result<Gid> {
        Ok(Gid::new(self.gid()))
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.set_gid(gid.into())
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

    fn page_cache(&self) -> Option<PageCache> {
        self.page_cache()
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn PerOpenFileOps>>> {
        match self.inode_type() {
            inode_type @ (InodeType::BlockDevice | InodeType::CharDevice) => {
                let device_id = self.device_id();
                let Some(device_id) = DeviceId::from_encoded_u64(device_id) else {
                    return Some(Err(Error::with_message(
                        Errno::ENODEV,
                        "the device ID is invalid",
                    )));
                };
                let device_type = inode_type
                    .device_type()
                    .expect("BlockDevice and CharDevice always have a device type");
                let Some(device) = device::lookup(device_type, device_id) else {
                    return Some(Err(Error::with_message(
                        Errno::ENODEV,
                        "the required device ID does not exist",
                    )));
                };

                Some(device.open())
            }
            InodeType::NamedPipe => {
                let pipe = self.pipe().expect("NamedPipe inode must have a pipe");
                Some(pipe.open_named(access_mode, status_flags))
            }
            _ => None,
        }
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Ok(self.create(name, type_, mode.into())?)
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        let (inode_type, device_id) = match type_ {
            MknodType::CharDevice(dev_id) => (InodeType::CharDevice, Some(dev_id)),
            MknodType::BlockDevice(dev_id) => (InodeType::BlockDevice, Some(dev_id)),
            MknodType::NamedPipe => (InodeType::NamedPipe, None),
        };

        let new_inode = self.create(name, inode_type, mode.into())?;
        if let Some(device_id) = device_id {
            // Store the ext2 special-file device encoding in `i_block`.
            new_inode.set_device_id(device_id)?;
        }

        Ok(new_inode)
    }

    fn lookup(&self, name: &str) -> Result<Arc<dyn Inode>> {
        Ok(self.lookup(name)?)
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

    fn rename(
        &self,
        old_name: &str,
        target: &Arc<dyn Inode>,
        new_name: &str,
        mode: RenameMode,
    ) -> Result<()> {
        if mode == RenameMode::Exchange {
            return_errno_with_message!(Errno::EINVAL, "RENAME_EXCHANGE is not supported on ext2");
        }

        let target = target
            .downcast_ref::<Ext2Inode>()
            .ok_or_else(|| Error::with_message(Errno::EXDEV, "not same fs"))?;
        self.rename(old_name, target, new_name)
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link().map(SymbolicLink::Plain)
    }

    fn write_link(&self, target: &str) -> Result<()> {
        self.write_link(target)
    }

    fn sync_all(&self) -> Result<()> {
        self.sync_all()?;
        let fs = self.fs()?;
        let block_group = fs.block_group(self.block_group_idx());
        block_group.sync_inode_table()?;
        if fs.block_device().sync()? != BioStatus::Complete {
            return_errno_with_message!(Errno::EIO, "failed to flush block device");
        }
        Ok(())
    }

    fn sync_data(&self) -> Result<()> {
        self.sync_data()?;
        let fs = self.fs()?;
        let block_group = fs.block_group(self.block_group_idx());
        block_group.sync_inode_table()?;
        if fs.block_device().sync()? != BioStatus::Complete {
            return_errno_with_message!(Errno::EIO, "failed to flush block device");
        }
        Ok(())
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.fallocate(mode, offset, len)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        // The inode must belong to a live filesystem instance.
        self.fs().unwrap()
    }

    fn extension(&self) -> &Extension {
        self.extension()
    }

    fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.check_permission(Permission::MAY_WRITE)?;
        self.set_xattr(name, value_reader, flags)
    }

    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        self.check_permission(Permission::MAY_READ)?;
        self.get_xattr(name, value_writer)
    }

    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        self.check_permission(Permission::MAY_ACCESS)?;
        self.list_xattr(namespace, list_writer)
    }

    fn remove_xattr(&self, name: XattrName) -> Result<()> {
        self.check_permission(Permission::MAY_WRITE)?;
        self.remove_xattr(name)
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
