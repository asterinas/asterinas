use core::time::Duration;
use jinux_frame::vm::VmFrame;

use crate::fs::utils::{
    DirentVisitor, FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata,
};
use crate::prelude::*;

use super::{ProcFS, ProcInodeInfo};

pub struct ProcFile<F: FileOps> {
    inner: F,
    info: ProcInodeInfo,
}

impl<F: FileOps> ProcFile<F> {
    pub fn new(file: F, fs: Arc<dyn FileSystem>, is_volatile: bool) -> Arc<Self> {
        let info = {
            let procfs = fs.downcast_ref::<ProcFS>().unwrap();
            let metadata = Metadata::new_file(
                procfs.alloc_id(),
                InodeMode::from_bits_truncate(0o444),
                &fs.sb(),
            );
            ProcInodeInfo::new(metadata, Arc::downgrade(&fs), is_volatile)
        };
        Arc::new(Self { inner: file, info })
    }
}

impl<F: FileOps + 'static> Inode for ProcFile<F> {
    fn len(&self) -> usize {
        self.info.metadata().size
    }

    fn resize(&self, _new_size: usize) {}

    fn metadata(&self) -> Metadata {
        self.info.metadata().clone()
    }

    fn atime(&self) -> Duration {
        self.info.metadata().atime
    }

    fn set_atime(&self, _time: Duration) {}

    fn mtime(&self) -> Duration {
        self.info.metadata().mtime
    }

    fn set_mtime(&self, _time: Duration) {}

    fn read_page(&self, _idx: usize, _frame: &VmFrame) -> Result<()> {
        unreachable!()
    }

    fn write_page(&self, _idx: usize, _frame: &VmFrame) -> Result<()> {
        unreachable!()
    }

    fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let data = self.inner.data()?;
        let start = data.len().min(offset);
        let end = data.len().min(offset + buf.len());
        let len = end - start;
        buf[0..len].copy_from_slice(&data[start..end]);
        Ok(len)
    }

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> Result<usize> {
        Err(Error::new(Errno::EPERM))
    }

    fn mknod(&self, _name: &str, _type_: InodeType, _mode: InodeMode) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn readdir_at(&self, _offset: usize, _visitor: &mut dyn DirentVisitor) -> Result<usize> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn link(&self, _old: &Arc<dyn Inode>, _name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn unlink(&self, _name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn rmdir(&self, _name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn lookup(&self, _name: &str) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn rename(&self, _old_name: &str, _target: &Arc<dyn Inode>, _new_name: &str) -> Result<()> {
        Err(Error::new(Errno::ENOTDIR))
    }

    fn read_link(&self) -> Result<String> {
        Err(Error::new(Errno::EINVAL))
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EINVAL))
    }

    fn ioctl(&self, _cmd: &IoctlCmd) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn sync(&self) -> Result<()> {
        Ok(())
    }

    fn fs(&self) -> Arc<dyn FileSystem> {
        self.info.fs().upgrade().unwrap()
    }

    fn is_dentry_cacheable(&self) -> bool {
        !self.info.is_volatile()
    }
}

pub trait FileOps: Sync + Send {
    fn data(&self) -> Result<Vec<u8>>;
}
