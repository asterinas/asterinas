// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use super::*;
use crate::{
    device::PtySlave,
    events::IoEvents,
    fs::inode_handle::FileIo,
    process::signal::{PollHandle, Pollable},
};

/// Same major number with Linux, the minor number is the index of slave.
const SLAVE_MAJOR_NUM: u32 = 3;

/// Pty slave inode for the slave device.
pub struct PtySlaveInode {
    device: Arc<PtySlave>,
    metadata: RwLock<Metadata>,
    fs: Weak<DevPts>,
}

impl PtySlaveInode {
    pub fn new(device: Arc<PtySlave>, fs: Weak<DevPts>) -> Arc<Self> {
        Arc::new(Self {
            metadata: RwLock::new(Metadata::new_device(
                device.index() as u64 + FIRST_SLAVE_INO,
                InodeMode::from_bits_truncate(0o620),
                super::BLOCK_SIZE,
                device.as_ref(),
            )),
            device,
            fs,
        })
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
        self.device.read(writer)
    }

    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.device.read(writer)
    }

    fn write_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.device.write(reader)
    }

    fn write_direct_at(&self, offset: usize, reader: &mut VmReader) -> Result<usize> {
        self.device.write(reader)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.device.ioctl(cmd, arg)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&mut PollHandle>) -> IoEvents {
        self.device.poll(mask, poller)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        Some(self.device.clone())
    }
}
