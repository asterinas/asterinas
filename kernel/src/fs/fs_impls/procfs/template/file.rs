// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use inherit_methods_macro::inherit_methods;

use super::Common;
use crate::{
    fs::{
        file::{AccessMode, FileIo, InodeMode, InodeType, StatusFlags},
        vfs::{
            file_system::FileSystem,
            inode::{Extension, Inode, InodeIo, Metadata, SymbolicLink},
        },
    },
    prelude::*,
    process::{Gid, Uid},
};

pub struct ProcFile<F: FileOps> {
    inner: F,
    common: Common,
}

impl<F: FileOps> ProcFile<F> {
    pub(super) fn new(
        file: F,
        fs: Weak<dyn FileSystem>,
        is_volatile: bool,
        mode: InodeMode,
    ) -> Arc<Self> {
        let common = new_file_common(fs, mode, is_volatile);
        Arc::new(Self {
            inner: file,
            common,
        })
    }

    pub fn inner(&self) -> &F {
        &self.inner
    }
}

fn new_file_common(fs: Weak<dyn FileSystem>, mode: InodeMode, is_volatile: bool) -> Common {
    let fs_ref = fs.upgrade().unwrap();
    let procfs = fs_ref.downcast_ref::<super::ProcFs>().unwrap();
    let metadata = Metadata::new_file(
        procfs.alloc_id(),
        mode,
        super::BLOCK_SIZE,
        procfs.sb().container_dev_id,
    );
    Common::new(metadata, fs, is_volatile)
}

impl<F: FileOps + 'static> InodeIo for ProcFile<F> {
    fn read_at(
        &self,
        offset: usize,
        writer: &mut VmWriter,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        self.inner.read_at(offset, writer)
    }

    fn write_at(
        &self,
        offset: usize,
        reader: &mut VmReader,
        _status_flags: StatusFlags,
    ) -> Result<usize> {
        self.inner.write_at(offset, reader)
    }
}

#[inherit_methods(from = "self.common")]
impl<F: FileOps + 'static> Inode for ProcFile<F> {
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
        // Resizing files under `/proc` will succeed, but will do nothing.
        Ok(())
    }

    fn type_(&self) -> InodeType {
        InodeType::File
    }

    fn read_link(&self) -> Result<SymbolicLink> {
        Err(Error::new(Errno::EINVAL))
    }

    fn write_link(&self, _target: &str) -> Result<()> {
        Err(Error::new(Errno::EINVAL))
    }

    fn is_dentry_cacheable(&self) -> bool {
        !self.common.is_volatile()
    }

    fn seek_end(&self) -> Option<usize> {
        // Seeking regular files under `/proc` with `SEEK_END` will fail.
        None
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        self.inner.open(access_mode, status_flags)
    }
}

pub trait FileOps: Sync + Send {
    fn read_at(&self, offset: usize, writer: &mut VmWriter) -> Result<usize>;

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        return_errno_with_message!(Errno::EPERM, "the file is not writable");
    }

    fn open(
        &self,
        _access_mode: AccessMode,
        _status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        None
    }
}

pub trait FileOpsByHandle: Sync + Send {
    fn open(&self, access_mode: AccessMode, status_flags: StatusFlags) -> Result<Box<dyn FileIo>>;
}

impl<T: FileOpsByHandle> FileOps for T {
    fn read_at(&self, _offset: usize, _writer: &mut VmWriter) -> Result<usize> {
        unreachable!("`read_at` is never called when `open` returns `Some(_)`")
    }

    fn write_at(&self, _offset: usize, _reader: &mut VmReader) -> Result<usize> {
        unreachable!("`write_at` is never called when `open` returns `Some(_)`")
    }

    fn open(
        &self,
        access_mode: AccessMode,
        status_flags: StatusFlags,
    ) -> Option<Result<Box<dyn FileIo>>> {
        Some(self.open(access_mode, status_flags))
    }
}
