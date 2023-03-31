//! Opend File Handle

mod file;
mod inode_handle;

use crate::fs::utils::{IoEvents, Metadata, Poller, SeekFrom};
use crate::prelude::*;
use crate::rights::{ReadOp, WriteOp};
use alloc::sync::Arc;

pub use self::file::File;
pub use self::inode_handle::InodeHandle;

#[derive(Clone)]
pub struct FileHandle {
    inner: Inner,
}

#[derive(Clone)]
enum Inner {
    File(Arc<dyn File>),
    Inode(InodeHandle),
}

impl FileHandle {
    pub fn new_file(file: Arc<dyn File>) -> Self {
        let inner = Inner::File(file);
        Self { inner }
    }

    pub fn new_inode_handle(inode_handle: InodeHandle) -> Self {
        let inner = Inner::Inode(inode_handle);
        Self { inner }
    }

    pub fn as_file(&self) -> Option<&Arc<dyn File>> {
        match &self.inner {
            Inner::File(file) => Some(file),
            _ => None,
        }
    }

    pub fn as_inode_handle(&self) -> Option<&InodeHandle> {
        match &self.inner {
            Inner::Inode(inode_handle) => Some(inode_handle),
            _ => None,
        }
    }

    pub fn read(&self, buf: &mut [u8]) -> Result<usize> {
        match &self.inner {
            Inner::File(file) => file.read(buf),
            Inner::Inode(inode_handle) => {
                let static_handle = inode_handle.clone().to_static::<ReadOp>()?;
                static_handle.read(buf)
            }
        }
    }

    pub fn write(&self, buf: &[u8]) -> Result<usize> {
        match &self.inner {
            Inner::File(file) => file.write(buf),
            Inner::Inode(inode_handle) => {
                let static_handle = inode_handle.clone().to_static::<WriteOp>()?;
                static_handle.write(buf)
            }
        }
    }

    pub fn metadata(&self) -> Metadata {
        match &self.inner {
            Inner::File(file) => file.metadata(),
            Inner::Inode(inode_handle) => inode_handle.dentry().vnode().metadata(),
        }
    }

    pub fn seek(&self, seek_from: SeekFrom) -> Result<usize> {
        match &self.inner {
            Inner::File(file) => file.seek(seek_from),
            Inner::Inode(inode_handle) => inode_handle.seek(seek_from),
        }
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        match &self.inner {
            Inner::File(file) => file.poll(mask, poller),
            Inner::Inode(inode_handle) => inode_handle.dentry().vnode().poll(mask, poller),
        }
    }

    pub fn clean_for_close(&self) -> Result<()> {
        match &self.inner {
            Inner::Inode(_) => {
                // Close does not guarantee that the data has been successfully saved to disk.
            }
            Inner::File(file) => file.flush()?,
        }
        Ok(())
    }
}
