// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(procfs)]

use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use super::Common;
use crate::{
    fs::{
        file::{InodeMode, InodeType, StatusFlags},
        procfs::{BLOCK_SIZE, ProcFs},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, FileOps, Inode, Metadata, SymbolicLink},
            path::Dentry,
        },
    },
    prelude::*,
    process::{Gid, Uid},
    thread::Thread,
};

pub(in procfs) struct ProcSym<S: ProcSymOps> {
    inner: S,
    common: Common,
}

impl<S: ProcSymOps> ProcSym<S> {
    pub(in procfs) fn new(sym: S, parent: Weak<dyn Inode>, mode: InodeMode) -> Arc<Self> {
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

    pub(in procfs) fn inner(&self) -> &S {
        &self.inner
    }
}

impl<S: ProcSymOps + 'static> FileOps for ProcSym<S> {
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
impl<S: ProcSymOps + 'static> Inode for ProcSym<S> {
    fn size(&self) -> usize;
    fn extension(&self) -> &Extension;
    fn ino(&self) -> u64;
    fn mode(&self) -> Result<InodeMode>;
    fn set_mode(&self, self_dentry: &Dentry, mode: InodeMode) -> Result<()>;
    fn owner(&self) -> Result<Uid>;
    fn set_owner(&self, self_dentry: &Dentry, uid: Uid) -> Result<()>;
    fn group(&self) -> Result<Gid>;
    fn set_group(&self, self_dentry: &Dentry, gid: Gid) -> Result<()>;
    fn atime(&self) -> Duration;
    fn set_atime(&self, self_dentry: &Dentry, time: Duration);
    fn mtime(&self) -> Duration;
    fn set_mtime(&self, self_dentry: &Dentry, time: Duration);
    fn ctime(&self) -> Duration;
    fn set_ctime(&self, self_dentry: &Dentry, time: Duration);
    fn fs(&self) -> Arc<dyn FileSystem>;

    fn metadata(&self) -> Result<Metadata> {
        let owner_thread = self.inner.owner_thread();
        Ok(self.common.metadata_with_owner(owner_thread))
    }

    fn resize(&self, _self_dentry: &Dentry, _new_size: usize) -> Result<()> {
        Err(Error::new(Errno::EPERM))
    }

    fn type_(&self) -> InodeType {
        InodeType::SymLink
    }

    fn create_symlink(
        &self,
        _self_dentry: &Dentry,
        _name: &str,
        _target: &str,
        _mode: InodeMode,
    ) -> Result<Arc<dyn Inode>> {
        Err(Error::new(Errno::EINVAL))
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        self.inner.read_link()
    }
}

pub(crate) trait ProcSymOps: Sync + Send {
    /// Returns the thread whose credentials own this procfs inode.
    fn owner_thread(&self) -> Option<Arc<Thread>> {
        None
    }

    fn read_link(&self) -> Result<SymbolicLink>;
}
