// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]
#![expect(unused_variables)]

use aster_device::{Device, DeviceId, DeviceType};

use super::*;
use crate::fs::device::DeviceFile;

/// Ptmx is the multiplexing master of devpts.
///
/// Every time the multiplexing master is opened, a new instance of pty master inode is returned
/// and an corresponding pty slave inode is also created.
pub struct Ptmx {
    inner: Inner,
    metadata: RwLock<Metadata>,
}

#[derive(Clone)]
struct Inner(Weak<DevPts>);

impl Ptmx {
    pub fn new(fs: Weak<DevPts>, device: Arc<PtmxDevice>) -> Arc<Self> {
        Arc::new(Self {
            metadata: RwLock::new(Metadata::new_device(
                PTMX_INO,
                mkmod!(a+rw),
                super::BLOCK_SIZE,
                device.as_ref(),
            )),
            inner: Inner(fs),
        })
    }

    /// The open method for ptmx.
    ///
    /// Creates a master and slave pair and returns the master inode.
    pub fn open(&self) -> Result<Arc<PtyMaster>> {
        let (master, _) = self.devpts().create_master_slave_pair()?;
        Ok(master)
    }

    pub fn devpts(&self) -> Arc<DevPts> {
        self.inner.0.upgrade().unwrap()
    }

    pub fn device_type(&self) -> DeviceType {
        self.devpts().ptmx().type_()
    }

    pub fn device_id(&self) -> DeviceId {
        self.devpts().ptmx().id().unwrap()
    }
}

// Many methods are left to do nothing because every time the ptmx is being opened,
// it returns the pty master. So the ptmx can not be used at upper layer.
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

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        Ok(0)
    }

    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        Ok(0)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        Ok(0)
    }

    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        Ok(0)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        Ok(0)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.devpts()
    }

    fn as_device(&self) -> Option<Arc<dyn DeviceFile>> {
        Some(self.devpts().ptmx().clone())
    }

    fn is_dentry_cacheable(&self) -> bool {
        false
    }
}
