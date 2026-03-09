// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]
#![expect(unused_variables)]

use super::*;
use crate::{
    device::PtySlave,
    fs::{
        inode_handle::FileIo,
        utils::{AccessMode, InodeIo, StatusFlags},
    },
};

/// Same major number with Linux, the minor number is the index of slave.
const SLAVE_MAJOR_NUM: u32 = 3;

/// Pty slave inode for the slave device.
pub struct PtySlaveInode {
    device: Arc<PtySlave>,
    metadata: RwLock<Metadata>,
    extension: Extension,
    fs: Weak<DevPts>,
}

impl PtySlaveInode {
    pub fn new(device: Arc<PtySlave>, fs: Weak<DevPts>) -> Arc<Self> {
        Arc::new(Self {
            metadata: RwLock::new(Metadata::new_device(
                device.index() as u64 + FIRST_SLAVE_INO,
                mkmod!(u+rw, g+w),
                super::BLOCK_SIZE,
                device.as_ref(),
            )),
            device,
            extension: Extension::new(),
            fs,
        })
    }
}

impl InodeIo for PtySlaveInode {
    fn read_at(
        &self,
        _offset: usize,
        writer: &mut VmWriter,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.device.read(writer, status_flags)
    }

    fn write_at(
        &self,
        _offset: usize,
        reader: &mut VmReader,
        status_flags: StatusFlags,
    ) -> Result<usize> {
        self.device.write(reader, status_flags)
    }
}

impl Inode for PtySlaveInode {
    /// Do not cache dentry in DCACHE.
    ///
    /// The slave will be deleted by the master when the master is released.
    /// So we should not cache the dentry.
    fn is_dentry_cacheable(&self) -> bool {
        false
    }

    fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn metadata(&self) -> Metadata {
        *self.metadata.read()
    }

    fn extension(&self) -> &Extension {
        &self.extension
    }

    fn ino(&self) -> u64 {
        self.metadata.read().ino as _
    }

    fn type_(&self) -> InodeType {
        self.metadata.read().type_
    }

    fn mode(&self) -> Result<InodeMode> {
        Ok(self.metadata.read().mode)
    }

    fn set_mode(&self, mode: InodeMode) -> Result<()> {
        self.metadata.write().mode = mode;
        Ok(())
    }

    fn owner(&self) -> Result<Uid> {
        Ok(self.metadata.read().uid)
    }

    fn set_owner(&self, uid: Uid) -> Result<()> {
        self.metadata.write().uid = uid;
        Ok(())
    }

    fn group(&self) -> Result<Gid> {
        Ok(self.metadata.read().gid)
    }

    fn set_group(&self, gid: Gid) -> Result<()> {
        self.metadata.write().gid = gid;
        Ok(())
    }

    fn atime(&self) -> Duration {
        self.metadata.read().atime
    }

    fn set_atime(&self, time: Duration) {
        self.metadata.write().atime = time;
    }

    fn mtime(&self) -> Duration {
        self.metadata.read().mtime
    }

    fn set_mtime(&self, time: Duration) {
        self.metadata.write().mtime = time;
    }

    fn ctime(&self) -> Duration {
        self.metadata.read().ctime
    }

    fn set_ctime(&self, time: Duration) {
        self.metadata.write().ctime = time;
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        Some(self.device.open())
    }
}
