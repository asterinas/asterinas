use crate::prelude::*;

use super::*;

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
        let index = self.0.slave_index();
        let _ = self.0.ptmx().devpts().remove_slave(index);
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

    fn read_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        Ok(())
    }

    fn write_page(&self, idx: usize, frame: &VmFrame) -> Result<()> {
        Ok(())
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.0.read(buf)
    }

    fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.0.write(buf)
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.0.ioctl(cmd, arg)
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.0.poll(mask, poller)
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.0.ptmx().devpts()
    }
}

// TODO: implement real pty master.
pub struct PtyMaster {
    slave_index: u32,
    ptmx: Arc<Ptmx>,
}

impl PtyMaster {
    pub fn new(slave_index: u32, ptmx: Arc<Ptmx>) -> Arc<Self> {
        Arc::new(Self { slave_index, ptmx })
    }

    pub fn slave_index(&self) -> u32 {
        self.slave_index
    }

    fn ptmx(&self) -> &Ptmx {
        &self.ptmx
    }
}

impl Device for PtyMaster {
    fn type_(&self) -> DeviceType {
        self.ptmx.device_type()
    }

    fn id(&self) -> DeviceId {
        self.ptmx.device_id()
    }

    fn read(&self, buf: &mut [u8]) -> Result<usize> {
        todo!();
    }

    fn write(&self, buf: &[u8]) -> Result<usize> {
        todo!();
    }

    fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        todo!();
    }

    fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        todo!();
    }
}
