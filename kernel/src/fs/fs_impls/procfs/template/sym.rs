// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use super::Common;
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags},
        procfs::{BLOCK_SIZE, ProcFs},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Inode, InodeIo, Metadata, SymbolicLink},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

pub struct ProcSym<S: SymOps> {
    inner: S,
    common: Common,
}

impl<S: SymOps> ProcSym<S> {
    pub fn new(sym: S, parent: Weak<dyn Inode>, mode: InodeMode) -> Arc<Self> {
        let common = {
            let fs = parent.upgrade().unwrap().fs();
            let procfs = fs.downcast_ref::<ProcFs>().unwrap();
            let metadata = Metadata::new_symlink(
                procfs.alloc_id(),
                mode,
                BLOCK_SIZE,
                procfs.sb().container_dev_id,
            );
            Common::new(metadata, Arc::downgrade(&fs))
        };
        Arc::new(Self { inner: sym, common })
    }

    pub fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: SymOps + 'static> InodeIo for ProcSym<S> {
    fn read_at(
        &self,
        _offset: usize,
        _writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(Error::new(Errno::EPERM))
    }

    fn write_at(
        &self,
        _offset: usize,
        _reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        Err(Error::new(Errno::EPERM))
    }
}

#[inherit_methods(from = "self.common")]
impl<S: SymOps + 'static> Inode for ProcSym<S> {
    fn size(&self) -> usize;
    fn metadata(&self) -> Metadata;
    fn extension(&self) -> &Extension;
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
        InodeType::SymLink
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.inner.read_link()
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }
}

pub trait SymOps: Sync + Send {
    fn read_link(&self) -> Result<SymbolicLink>;
}
