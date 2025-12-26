// SPDX-License-Identifier: MPL-2.0

use device_id::{DeviceId, MajorId, MinorId};

use super::*;
use crate::fs::{
    inode_handle::FileIo,
    utils::{AccessMode, Extension, InodeIo, StatusFlags},
};

/// Same major number with Linux.
const PTMX_MAJOR_NUM: u16 = 5;
/// Same minor number with Linux.
const PTMX_MINOR_NUM: u32 = 2;

/// Ptmx is the multiplexing master of devpts.
///
/// Every time the multiplexing master is opened, a new instance of pty master inode is returned
/// and an corresponding pty slave inode is also created.
pub struct Ptmx {
    inner: Inner,
    metadata: RwLock<Metadata>,
    extension: Extension,
}

#[derive(Clone)]
struct Inner(Weak<DevPts>);

impl Ptmx {
    pub fn new(fs: Weak<DevPts>) -> Arc<Self> {
        let inner = Inner(fs);
        Arc::new(Self {
            metadata: RwLock::new(Metadata::new_device(
                PTMX_INO,
                mkmod!(a+rw),
                super::BLOCK_SIZE,
                &inner,
            )),
            inner,
            extension: Extension::new(),
        })
    }

    pub fn devpts(&self) -> Option<Arc<DevPts>> {
        self.inner.0.upgrade()
    }
}

// Many methods are left to do nothing because every time the ptmx is being opened,
// it returns the pty master. So the ptmx can not be used at upper layer.
impl InodeIo for Ptmx {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Ok(0)
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Ok(0)
    }
}

impl Inode for Ptmx {
    fn size(&self) -> usize {
        self.metadata.read().size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        Ok(())
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
        // FIXME: The below code may panic if the devpts is dropped.
        self.devpts().unwrap()
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        Some(self.inner.open())
    }

    fn is_dentry_cacheable(&self) -> bool {
        false
    }
}

impl Device for Inner {
    fn type_(&self) -> DeviceType {
        DeviceType::Char
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(MajorId::new(PTMX_MAJOR_NUM), MinorId::new(PTMX_MINOR_NUM))
    }

    fn devtmpfs_path(&self) -> Option<String> {
        None
    }

    fn open(&self) -> Result<Box<dyn FileIo>> {
        let devpts = self.0.upgrade().unwrap();
        Ok(devpts.create_master_slave_pair()?.0)
    }
}
