// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use device_id::DeviceId;

use crate::{
    device,
    fs::{
        ext2::{FilePerm, Inode as Ext2Inode},
        inode_handle::FileIo,
        utils::{
            AccessMode, DirentVisitor, Extension, FallocMode, FileSystem, Inode, InodeIo,
            InodeMode, InodeType, Metadata, MknodType, StatusFlags, SymbolicLink, XattrName,
            XattrNamespace, XattrSetFlags,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    vm::vmo::Vmo,
};

impl InodeIo for Ext2Inode {
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
}

impl Inode for Ext2Inode {
    fn size(&self) -> usize {
        self.file_size() as _
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        self.resize(new_size)
    }

    fn metadata(&self) -> Metadata {
        self.metadata()
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

    fn page_cache(&self) -> Option<Arc<Vmo>> {
        Some(self.page_cache())
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        match self.inode_type() {
            inode_type @ (InodeType::BlockDevice | InodeType::CharDevice) => {
                let device_id = self.device_id();
                let Some(device_id) = DeviceId::from_encoded_u64(device_id) else {
                    return Some(Err(Error::with_message(
                        Errno::ENODEV,
                        "the device ID is invalid",
                    )));
                };
                let device_type = inode_type.device_type().unwrap();
                let Some(device) = device::lookup(device_type, device_id) else {
                    return Some(Err(Error::with_message(
                        Errno::ENODEV,
                        "the required device ID does not exist",
                    )));
                };

                Some(device.open())
            }
            InodeType::NamedPipe => {
                let pipe = self.named_pipe().unwrap();

                Some(pipe.open_named(access_mode, status_flags))
            }
            _ => None,
        }
    }

    fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Ok(self.create(name, type_, mode.into())?)
    }

    fn mknod(&self, name: &str, mode: InodeMode, type_: MknodType) -> Result<Arc<dyn Inode>> {
        let inode = match type_ {
            MknodType::CharDevice(dev) => {
                let inode = self.create(name, InodeType::CharDevice, mode.into())?;
                inode.set_device_id(dev).unwrap();
                inode
            }
            MknodType::BlockDevice(dev) => {
                let inode = self.create(name, InodeType::BlockDevice, mode.into())?;
                inode.set_device_id(dev).unwrap();
                inode
            }
            MknodType::NamedPipe => self.create(name, InodeType::NamedPipe, mode.into())?,
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

    fn read_link(&self) -> Result<SymbolicLink> {
        self.read_link().map(SymbolicLink::Plain)
    }

    fn write_link(&self, target: &str) -> Result<()> {
        self.write_link(target)
    }

    fn fallocate(&self, mode: FallocMode, offset: usize, len: usize) -> Result<()> {
        self.fallocate(mode, offset, len)
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

    fn extension(&self) -> &Extension {
        self.extension()
    }

    fn set_xattr(
        &self,
        name: XattrName,
        value_reader: &mut VmReader,
        flags: XattrSetFlags,
    ) -> Result<()> {
        self.set_xattr(name, value_reader, flags)
    }

    fn get_xattr(&self, name: XattrName, value_writer: &mut VmWriter) -> Result<usize> {
        self.get_xattr(name, value_writer)
    }

    fn list_xattr(&self, namespace: XattrNamespace, list_writer: &mut VmWriter) -> Result<usize> {
        self.list_xattr(namespace, list_writer)
    }

    fn remove_xattr(&self, name: XattrName) -> Result<()> {
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
