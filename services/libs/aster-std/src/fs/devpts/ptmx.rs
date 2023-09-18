use crate::device::PtyMaster;
use crate::events::IoEvents;
use crate::fs::inode_handle::FileIo;
use crate::prelude::*;
use crate::process::signal::Poller;

use super::*;

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
    metadata: Metadata,
}

#[derive(Clone)]
struct Inner(Weak<DevPts>);

impl Ptmx {
    pub fn new(sb: &SuperBlock, fs: Weak<DevPts>) -> Arc<Self> {
        let inner = Inner(fs);
        Arc::new(Self {
            metadata: Metadata::new_device(
                PTMX_INO,
                InodeMode::from_bits_truncate(0o666),
                sb,
                &inner,
            ),
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
    fn len(&self) -> usize {
        self.metadata.size
    }

    fn resize(&self, new_size: usize) -> Result<()> {
        Ok(())
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
        Ok(0)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        Ok(0)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        Ok(0)
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
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

impl FileIo for Inner {
    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot read ptmx");
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        return_errno_with_message!(Errno::EINVAL, "cannot write ptmx");
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        IoEvents::empty()
    }
}
