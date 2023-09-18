use crate::events::IoEvents;
use crate::fs::inode_handle::FileIo;
use crate::prelude::*;
use crate::process::signal::Poller;

use super::*;

use crate::device::PtySlave;

/// Same major number with Linux, the minor number is the index of slave.
const SLAVE_MAJOR_NUM: u32 = 3;

/// Pty slave inode for the slave device.
pub struct PtySlaveInode {
    device: Arc<PtySlave>,
    metadata: Metadata,
    fs: Weak<DevPts>,
}

impl PtySlaveInode {
    pub fn new(device: Arc<PtySlave>, fs: Weak<DevPts>) -> Arc<Self> {
        Arc::new(Self {
            metadata: Metadata::new_device(
                device.index() as usize + FIRST_SLAVE_INO,
                InodeMode::from_bits_truncate(0o620),
                &fs.upgrade().unwrap().sb(),
                device.as_ref(),
            ),
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

    fn len(&self) -> usize {
        self.metadata.size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn metadata(&self) -> Metadata {
        self.metadata.clone()
    }

    fn ino(&self) -> u64 {
        self.metadata.ino as _
    }

    fn type_(&self) -> InodeType {
        self.metadata.type_
    }

    fn mode(&self) -> InodeMode {
        self.metadata.mode
    }

    fn set_mode(&self, mode: InodeMode) {}

    fn atime(&self) -> Duration {
        self.metadata.atime
    }

    fn set_atime(&self, time: Duration) {}

    fn mtime(&self) -> Duration {
        self.metadata.mtime
    }

    fn set_mtime(&self, time: Duration) {}

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.device.read(buf)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.device.read(buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.device.write(buf)
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.device.write(buf)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.device.ioctl(cmd, arg)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.device.poll(mask, poller)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.fs.upgrade().unwrap()
    }

    fn as_device(&self) -> Option<Arc<dyn Device>> {
        Some(self.device.clone())
    }
}
