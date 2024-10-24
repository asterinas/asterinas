// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use super::*;
use crate::{
    events::IoEvents,
    fs::inode_handle::FileIo,
    process::signal::{PollHandle, Pollable},
};

/// Same major number with Linux.
const PTMX_MAJOR_NUM: u32 = 5;
/// Same minor number with Linux.
const PTMX_MINOR_NUM: u32 = 2;

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
    pub fn new(fs: Weak<DevPts>) -> Arc<Self> {
        let inner = Inner(fs);
        Arc::new(Self {
            metadata: RwLock::new(Metadata::new_device(
                PTMX_INO,
                InodeMode::from_bits_truncate(0o666),
                super::BLOCK_SIZE,
                &inner,
            )),
            inner,
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
        self.inner.type_()
    }

    pub fn device_id(&self) -> DeviceId {
        self.inner.id()
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

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        Some(Arc::new(self.inner.clone()))
    }
}

impl Device for Inner {
    fn type_(&self) -> DeviceType {
        DeviceType::CharDevice
    }

    fn id(&self) -> DeviceId {
        DeviceId::new(PTMX_MAJOR_NUM, PTMX_MINOR_NUM)
    }

    fn open(&self) -> Result<Option<Arc<dyn FileIo>>> {
        let devpts = self.0.upgrade().unwrap();
        let (master, _) = devpts.create_master_slave_pair()?;
        Ok(Some(master as _))
    }
}

impl Pollable for Inner {
    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        IoEvents::empty()
    }
}

impl FileIo for Inner {
    fn read(&self, writer: &mut VmWriter) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read ptmx");
    }

    fn write(&self, reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write ptmx");
    }
}
