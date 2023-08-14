use super::{
    DirentVisitor, FileSystem, FsFlags, Inode, InodeMode, InodeType, IoEvents, IoctlCmd, Metadata,
    PageCache, Poller,
};
use crate::fs::device::Device;
use crate::prelude::*;
use crate::vm::vmo::Vmo;

use alloc::string::String;
use core::time::Duration;
use core2::io::{Error as IoError, ErrorKind as IoErrorKind, Result as IoResult, Write};
use jinux_frame::vm::VmIo;
use jinux_rights::Full;

/// VFS-level representation of an inode
#[derive(Clone)]
pub struct Vnode {
    // The RwLock is to maintain the correct file length for concurrent read or write.
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    inode: Arc<dyn Inode>,
    page_cache: Option<PageCache>,
}

impl Vnode {
    pub fn page_cache(&self) -> Option<Vmo<Full>> {
        self.inner
            .read()
            .page_cache
            .as_ref()
            .map(|page_chche| page_chche.pages().dup().unwrap())
    }

    pub fn new(inode: Arc<dyn Inode>) -> Result<Self> {
        let page_cache = if inode.fs().flags().contains(FsFlags::NO_PAGECACHE) {
            None
        } else {
            Some(PageCache::new(&inode)?)
        };
        Ok(Self {
            inner: Arc::new(RwLock::new(Inner { inode, page_cache })),
        })
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_write() {
            return_errno!(Errno::EINVAL);
        }
        let inner = self.inner.write();
        match &inner.page_cache {
            None => inner.inode.write_at(offset, buf),
            Some(page_cache) => {
                let file_len = inner.inode.len();
                let should_expand_len = offset + buf.len() > file_len;
                if should_expand_len {
                    page_cache.pages().resize(offset + buf.len())?;
                }
                page_cache.pages().write_bytes(offset, buf)?;
                if should_expand_len {
                    inner.inode.resize(offset + buf.len());
                }
                Ok(buf.len())
            }
        }
    }

    pub fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_write() {
            return_errno!(Errno::EINVAL);
        }
        let inner = self.inner.write();
        if let Some(page_cache) = &inner.page_cache {
            page_cache.evict_range(offset..offset + buf.len());
        }
        inner.inode.write_at(offset, buf)
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        match &inner.page_cache {
            None => inner.inode.read_at(offset, buf),
            Some(page_cache) => {
                let (offset, read_len) = {
                    let file_len = inner.inode.len();
                    let start = file_len.min(offset);
                    let end = file_len.min(offset + buf.len());
                    (start, end - start)
                };
                page_cache
                    .pages()
                    .read_bytes(offset, &mut buf[..read_len])?;
                Ok(read_len)
            }
        }
    }

    pub fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        if let Some(page_cache) = &inner.page_cache {
            page_cache.evict_range(offset..offset + buf.len());
        }
        inner.inode.read_at(offset, buf)
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        let file_len = inner.inode.len();
        if buf.len() < file_len {
            buf.resize(file_len, 0);
        }
        match &inner.page_cache {
            None => inner.inode.read_at(0, &mut buf[..file_len]),
            Some(page_cache) => {
                page_cache.pages().read_bytes(0, &mut buf[..file_len])?;
                Ok(file_len)
            }
        }
    }

    pub fn read_direct_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let type_ = self.inode_type();
        if !type_.support_read() {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        let file_len = inner.inode.len();
        if buf.len() < file_len {
            buf.resize(file_len, 0);
        }
        if let Some(page_cache) = &inner.page_cache {
            page_cache.evict_range(0..file_len);
        }

        inner.inode.read_at(0, &mut buf[..file_len])
    }

    pub fn create(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Self> {
        let inner = self.inner.read();
        let inode = match &inner.page_cache {
            None => inner.inode.create(name, type_, mode)?,
            Some(page_cache) => {
                inner
                    .inode
                    .create_with_pages(name, type_, mode, page_cache.pages())?
            }
        };
        Self::new(inode)
    }

    pub fn mknod(&self, name: &str, mode: InodeMode, device: Arc<dyn Device>) -> Result<Self> {
        let inner = self.inner.read();
        let inode = match &inner.page_cache {
            None => inner.inode.mknod(name, mode, device)?,
            Some(page_cache) => {
                inner
                    .inode
                    .mknod_with_pages(name, mode, device, page_cache.pages())?
            }
        };
        Self::new(inode)
    }

    pub fn lookup(&self, name: &str) -> Result<Self> {
        let inner = self.inner.read();
        let inode = match &inner.page_cache {
            None => inner.inode.lookup(name)?,
            Some(page_cache) => inner.inode.lookup_with_pages(name, page_cache.pages())?,
        };
        Self::new(inode)
    }

    pub fn link(&self, old: &Vnode, name: &str) -> Result<()> {
        let inner = self.inner.read();
        match &inner.page_cache {
            None => inner.inode.link(&old.inner.read().inode, name),
            Some(page_cache) => {
                inner
                    .inode
                    .link_with_pages(&old.inner.read().inode, name, page_cache.pages())
            }
        }
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        let inner = self.inner.read();
        match &inner.page_cache {
            None => inner.inode.unlink(name),
            Some(page_cache) => inner.inode.unlink_with_pages(name, page_cache.pages()),
        }
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        let inner = self.inner.read();
        match &inner.page_cache {
            None => inner.inode.rmdir(name),
            Some(page_cache) => inner.inode.rmdir_with_pages(name, page_cache.pages()),
        }
    }

    pub fn rename(&self, old_name: &str, target: &Vnode, new_name: &str) -> Result<()> {
        let inner = self.inner.read();
        let target_inner = target.inner.read();
        match (&inner.page_cache, &target_inner.page_cache) {
            (None, None) => inner.inode.rename(old_name, &target_inner.inode, new_name),
            (Some(self_pages), Some(target_pages)) => inner.inode.rename_with_pages(
                old_name,
                self_pages.pages(),
                &target_inner.inode,
                new_name,
                target_pages.pages(),
            ),
            _ => {
                return_errno!(Errno::EXDEV);
            }
        }
    }

    pub fn read_link(&self) -> Result<String> {
        let inner = self.inner.read();
        match &inner.page_cache {
            None => inner.inode.read_link(),
            Some(page_cache) => inner.inode.read_link_with_pages(page_cache.pages()),
        }
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        let inner = self.inner.write();
        match &inner.page_cache {
            None => inner.inode.write_link(target),
            Some(page_cache) => {
                inner.inode.resize(target.len());
                page_cache
                    .pages()
                    .resize(inner.inode.blocks_len().min(inner.inode.len()))?;
                inner
                    .inode
                    .write_link_with_pages(target, page_cache.pages())
            }
        }
    }

    pub fn readdir_at(&self, offset: usize, visitor: &mut dyn DirentVisitor) -> Result<usize> {
        let inner = self.inner.read();
        match &inner.page_cache {
            None => inner.inode.readdir_at(offset, visitor),
            Some(page_cache) => {
                inner
                    .inode
                    .readdir_at_with_pages(offset, visitor, page_cache.pages())
            }
        }
    }

    pub fn poll(&self, mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let inode = self.inner.read().inode.clone();
        inode.poll(mask, poller)
    }

    pub fn ioctl(&self, cmd: IoctlCmd, arg: usize) -> Result<i32> {
        let inode = self.inner.read().inode.clone();
        inode.ioctl(cmd, arg)
    }

    pub fn fs(&self) -> Arc<dyn FileSystem> {
        self.inner.read().inode.fs()
    }

    pub fn metadata(&self) -> Metadata {
        self.inner.read().inode.metadata()
    }

    pub fn inode(&self) -> Weak<dyn Inode> {
        let inner = self.inner.read();
        Arc::downgrade(&inner.inode)
    }

    pub fn inode_type(&self) -> InodeType {
        self.inner.read().inode.metadata().type_
    }

    pub fn inode_mode(&self) -> InodeMode {
        self.inner.read().inode.metadata().mode
    }

    pub fn set_inode_mode(&self, mode: InodeMode) {
        self.inner.read().inode.set_mode(mode)
    }

    pub fn len(&self) -> usize {
        self.inner.read().inode.len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }

    pub fn atime(&self) -> Duration {
        self.inner.read().inode.atime()
    }

    pub fn set_atime(&self, time: Duration) {
        self.inner.read().inode.set_atime(time)
    }

    pub fn mtime(&self) -> Duration {
        self.inner.read().inode.mtime()
    }

    pub fn set_mtime(&self, time: Duration) {
        self.inner.read().inode.set_mtime(time)
    }

    pub fn is_dentry_cacheable(&self) -> bool {
        self.inner.read().inode.is_dentry_cacheable()
    }

    pub fn writer(&self, from_offset: usize) -> VnodeWriter {
        VnodeWriter {
            inner: self,
            offset: from_offset,
        }
    }
}

impl Debug for Vnode {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Vnode")
            .field("inode", &self.inner.read().inode)
            .field("page_cache", &self.inner.read().page_cache)
            .finish()
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
