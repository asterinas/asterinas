//! Opend File Handle

mod file;
mod inode_handle;

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
}
