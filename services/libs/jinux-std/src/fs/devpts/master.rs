use crate::{fs::file_handle::FileLike, prelude::*};

use super::*;

use crate::device::PtyMaster;

/// Pty master inode for the master device.
pub struct PtyMasterInode(Arc<PtyMaster>);

impl PtyMasterInode {
    pub fn new(device: Arc<PtyMaster>) -> Arc<Self> {
        Arc::new(Self(device))
    }
}

impl Drop for PtyMasterInode {
    fn drop(&mut self) {
        // Remove the slave from fs.
        let fs = self.0.ptmx().fs();
        let devpts = fs.downcast_ref::<DevPts>().unwrap();

        let index = self.0.index();
        devpts.remove_slave(index);
    }
}

impl Inode for PtyMasterInode {
    /// Do not cache dentry in DCACHE.
    ///
    /// Each file descriptor obtained by opening "/dev/ptmx" is an independent pty master
    /// with its own associated pty slave.
    fn is_dentry_cacheable(&self) -> bool {
        false
    }

    fn len(&self) -> usize {
        self.0.ptmx().metadata().size
    }

    fn resize(&self, new_size: usize) {}

    fn metadata(&self) -> Metadata {
        self.0.ptmx().metadata()
    }

    fn atime(&self) -> Duration {
        self.0.ptmx().metadata().atime
    }

    fn set_atime(&self, time: Duration) {}

    fn mtime(&self) -> Duration {
        self.0.ptmx().metadata().mtime
    }

    fn set_mtime(&self, time: Duration) {}

    fn set_mode(&self, mode: InodeMode) {}

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf)
    }

    fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.0.write(buf)
    }

    fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.0.write(buf)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.0.ioctl(cmd, arg)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.0.poll(mask, poller)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.0.ptmx().fs()
    }
}
