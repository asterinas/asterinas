use super::{
    DirentVisitor, FileSystem, Inode, InodeMode, InodeType, IoEvents, IoctlCmd, Metadata, Poller,
};
use crate::fs::device::Device;
use crate::prelude::*;
use crate::vm::vmo::Vmo;

use alloc::string::String;
use core::time::Duration;
use core2::io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult, Write};
use jinux_rights::Full;

/// VFS-level representation of an inode
#[derive(Clone, Debug)]
pub struct Vnode {
    inode: Arc<dyn Inode>,
}

impl Vnode {
    pub fn new(inode: Arc<dyn Inode>) -> Self {
        Self { inode }
    }

    pub fn page_cache(&self) -> Option<Vmo<Full>> {
        self.inode.page_cache()
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_write() {
            return_errno!(Errno::EISDIR);
        }

        self.inode.write_at(offset, buf)
    }

    pub fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_write() {
            return_errno!(Errno::EISDIR);
        }

        self.inode.write_direct_at(offset, buf)
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }

        self.inode.read_at(offset, buf)
    }

    pub fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }

        self.inode.read_direct_at(offset, buf)
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }

        let file_len = self.len();
        if buf.len() < file_len {
            buf.resize(file_len, 0);
        }
        self.inode.read_at(0, &mut buf[..file_len])
    }

    pub fn read_direct_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }

        let file_len = self.len();
        if buf.len() < file_len {
            buf.resize(file_len, 0);
        }
        self.inode.read_direct_at(0, &mut buf[..file_len])
    }

    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Self> {
        let inode = self.inode.create(name, type_, mode)?;
        Ok(Self::new(inode))
    }

    pub fn mknod(&self, name: &str, mode: InodeMode, device: Arc<dyn Device>) -> Result<Self> {
        let inode = self.inode.mknod(name, mode, device)?;
        Ok(Self::new(inode))
    }

    pub fn lookup(&self, name: &str) -> Result<Self> {
        let inode = self.inode.lookup(name)?;
        Ok(Self::new(inode))
    }

    pub fn link(&self, old: &Vnode, name: &str) -> Result<()> {
        self.inode.link(&old.inode, name)
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        self.inode.unlink(name)
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        self.inode.rmdir(name)
    }

    pub fn rename(&self, old_name: &str, target: &Vnode, new_name: &str) -> Result<()> {
        if !Arc::ptr_eq(&self.fs(), &target.fs()) {
            return_errno_with_message!(Errno::EXDEV, "not same fs");
        }

        self.inode.rename(old_name, &target.inode, new_name)
    }

    pub fn read_link(&self) -> Result<String> {
        self.inode.read_link()
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        self.inode.write_link(target)
    }

    pub fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        self.inode.readdir_at(offset, visitor)
    }

    fn sync(&self) -> Result<()> {
        self.inode.sync()
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        self.inode.poll(mask, poller)
    }

    pub fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        self.inode.ioctl(cmd, arg)
    }

    pub fn fs(&self) -> Arc<dyn FileSystem> {
        self.inode.fs()
    }

    pub fn metadata(&self) -> Metadata {
        self.inode.metadata()
    }

    pub fn inode(&self) -> Weak<dyn Inode> {
        Arc::downgrade(&self.inode)
    }

    pub fn inode_type(&self) -> InodeType {
        self.inode.metadata().type_
    }

    pub fn inode_mode(&self) -> InodeMode {
        self.inode.metadata().mode
    }

    pub fn set_inode_mode(&self, mode: InodeMode) {
        self.inode.set_mode(mode)
    }

    pub fn len(&self) -> usize {
        self.inode.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn atime(&self) -> Duration {
        self.inode.atime()
    }

    pub fn set_atime(&self, time: Duration) {
        self.inode.set_atime(time)
    }

    pub fn mtime(&self) -> Duration {
        self.inode.mtime()
    }

    pub fn set_mtime(&self, time: Duration) {
        self.inode.set_mtime(time)
    }

    pub fn is_dentry_cacheable(&self) -> bool {
        self.inode.is_dentry_cacheable()
    }

    pub fn writer(&self, from_offset: usize) -> VnodeWriter {
        VnodeWriter {
            inner: self,
            offset: from_offset,
        }
    }
}

impl Drop for Vnode {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inode) == 1 {
            self.inode.sync().unwrap();
        }
    }
}

pub struct VnodeWriter<'a> {
    inner: &'a Vnode,
    offset: usize,
}

impl<'a> Write for VnodeWriter<'a> {
    #[inline]
    fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
        let write_len = self
            .inner
            .write_at(self.offset, buf)
            .map_err(|_| IoError::new(IoErrorKind::WriteZero, "failed to write buffer"))?;
        self.offset += write_len;
        Ok(write_len)
    }

    #[inline]
    fn flush(&mut self) -> IoResult<()> {
        Ok(())
    }
}
