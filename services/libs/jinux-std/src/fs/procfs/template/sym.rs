use core::time::Duration;
use jinux_frame::vm::VmFrame;

use crate::fs::utils::{FileSystem, Inode, InodeMode, IoctlCmd, Metadata};
use crate::prelude::*;

use super::{ProcFS, ProcInodeInfo};

pub struct ProcSym<S: SymOps> {
    inner: S,
    info: ProcInodeInfo,
}

impl<S: SymOps> ProcSym<S> {
    pub fn new(sym: S, fs: Arc<dyn FileSystem>, is_volatile: bool) -> Arc<Self> {
        let info = {
            let procfs = fs.downcast_ref::<ProcFS>().unwrap();
            let metadata = Metadata::new_symlink(
                procfs.alloc_id(),
                InodeMode::from_bits_truncate(0o777),
                &fs.sb(),
            );
            ProcInodeInfo::new(metadata, Arc::downgrade(&fs), is_volatile)
        };
        Arc::new(Self { inner: sym, info })
    }
}

impl<S: SymOps + 'static> Inode for ProcSym<S> {
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
        Err(Error::new(Errno::EPERM))
    }

    fn write_page(&self, _idx: usize, _frame: &VmFrame) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn read_at(&self, _offset: usize, _buf: &mut [u8]) -> Result<usize> {
        Err(Error::new(Errno::EPERM))
    }

    fn write_at(&self, _offset: usize, _buf: &[u8]) -> Result<usize> {
        Err(Error::new(Errno::EPERM))
    }

    fn read_link(&self) -> Result<String> {
        self.inner.read_link()
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
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

pub trait SymOps: Sync + Send {
    fn read_link(&self) -> Result<String>;
}
