// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use super::{Common, ProcFS};
use crate::{
    fs::utils::{FileSystem, Inode, InodeMode, InodeType, IoctlCmd, Metadata},
    prelude::*,
    process::{Gid, Uid},
};

pub struct ProcFile<F: FileOps> {
    inner: F,
    common: Common,
}

impl<F: FileOps> ProcFile<F> {
    pub fn new(file: F, fs: Weak<dyn FileSystem>, is_volatile: bool) -> Arc<Self> {
        let common = {
            let arc_fs = fs.upgrade().unwrap();
            let procfs = arc_fs.downcast_ref::<ProcFS>().unwrap();
            let metadata = Metadata::new_file(
                procfs.alloc_id(),
                InodeMode::from_bits_truncate(0o444),
                super::BLOCK_SIZE,
            );
            Common::new(metadata, fs, is_volatile)
        };
        Arc::new(Self {
            inner: file,
            common,
        })
    }
}

#[inherit_methods(from = "self.common")]
impl<F: FileOps + 'static> Inode for ProcFile<F> {
    fn size(&self) -> usize;
    fn metadata(&self) -> Metadata;
    fn ino(&self) -> u64;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;

    fn resize(&self, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn type_(&self) -> InodeType {
        InodeType::File
    }

    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        let data = self.inner.data()?;
        let start = data.len().min(offset);
        let end = data.len().min(offset + writer.avail());
        let len = end - start;
        writer.write_fallible(&mut (&data[start..end]).into())?;
        Ok(len)
    }

    fn read_direct_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize> {
        self.read_at(offset, writer)
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::new(Errno::EPERM))
    }

    fn write_direct_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        Err(Error::new(Errno::EPERM))
    }

    fn read_link(&self) -> Result<String> {
        Err(Error::new(Errno::EINVAL))
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EINVAL))
    }

    fn ioctl(&self, _cmd: IoctlCmd, _arg: usize) -> Result<i32> {
        Err(Error::new(Errno::EPERM))
    }

    fn is_dentry_cacheable(&self) -> bool {
        !self.common.is_volatile()
    }
}

pub trait FileOps: Sync + Send {
    fn data(&self) -> Result<Vec<u8>>;
}
