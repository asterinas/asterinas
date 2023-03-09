use super::{DirentWriterContext, Inode, InodeMode, InodeType, Metadata, PageCacheManager};
use crate::prelude::*;
use crate::rights::Rights;
use crate::vm::vmo::{Vmo, VmoFlags, VmoOptions};
use alloc::string::String;
use core::time::Duration;
use jinux_frame::vm::VmIo;

/// VFS-level representation of an inode
#[derive(Clone)]
pub struct Vnode {
    // The RwLock is to maintain the correct file length for concurrent read or write.
    inner: Arc<RwLock<Inner>>,
}

struct Inner {
    inode: Arc<dyn Inode>,
    page_cache: Vmo,
    page_cache_manager: Arc<PageCacheManager>,
}

impl Vnode {
    pub fn new(inode: Arc<dyn Inode>) -> Result<Self> {
        let page_cache_manager = Arc::new(PageCacheManager::new(&Arc::downgrade(&inode)));
        let page_cache = VmoOptions::<Rights>::new(inode.len())
            .flags(VmoFlags::RESIZABLE)
            .pager(page_cache_manager.clone())
            .alloc()?;
        Ok(Self {
            inner: Arc::new(RwLock::new(Inner {
                inode,
                page_cache,
                page_cache_manager,
            })),
        })
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if type_ != InodeType::File && type_ != InodeType::Socket {
            return_errno!(Errno::EINVAL);
        }
        let inner = self.inner.write();
        let file_len = inner.inode.len();
        let should_expand_len = offset + buf.len() > file_len;
        if should_expand_len {
            inner.page_cache.resize(offset + buf.len())?;
        }
        inner.page_cache.write_bytes(offset, buf)?;
        if should_expand_len {
            inner.inode.resize(offset + buf.len());
        }
        Ok(buf.len())
    }

    pub fn write_direct_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if type_ != InodeType::File && type_ != InodeType::Socket {
            return_errno!(Errno::EINVAL);
        }
        let inner = self.inner.write();
        let file_len = inner.inode.len();
        if offset + buf.len() > file_len {
            inner.page_cache.resize(offset + buf.len())?;
        }
        // Flush the dirty pages if necessary.
        // inner.page_cache_manager.flush(offset..offset + buf.len())?;
        // TODO: Update the related page state to invalid to reload the content from inode
        //       for upcoming read or write.
        inner.inode.write_at(offset, buf)
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if type_ != InodeType::File && type_ != InodeType::Socket {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        let (offset, read_len) = {
            let file_len = inner.inode.len();
            let start = file_len.min(offset);
            let end = file_len.min(offset + buf.len());
            (start, end - start)
        };
        inner.page_cache.read_bytes(offset, &mut buf[..read_len])?;
        Ok(read_len)
    }

    pub fn read_direct_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let type_ = self.inode_type();
        if type_ != InodeType::File && type_ != InodeType::Socket {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        // Flush the dirty pages if necessary.
        // inner.page_cache_manager.flush(offset..offset + buf.len())?;
        inner.inode.read_at(offset, buf)
    }

    pub fn read_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let type_ = self.inode_type();
        if type_ != InodeType::File && type_ != InodeType::Socket {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        let file_len = inner.inode.len();
        if buf.len() < file_len {
            buf.resize(file_len, 0);
        }
        inner.page_cache.read_bytes(0, &mut buf[..file_len])?;
        Ok(file_len)
    }

    pub fn read_direct_to_end(&self, buf: &mut Vec<u8>) -> Result<usize> {
        let type_ = self.inode_type();
        if type_ != InodeType::File && type_ != InodeType::Socket {
            return_errno!(Errno::EISDIR);
        }
        let inner = self.inner.read();
        let file_len = inner.inode.len();
        if buf.len() < file_len {
            buf.resize(file_len, 0);
        }
        // Flush the dirty pages if necessary.
        // inner.page_cache_manager.flush(..file_size)?;
        let len = inner.inode.read_at(0, &mut buf[..file_len])?;
        Ok(len)
    }

    pub fn mknod(&self, name: &str, type_: InodeType, mode: InodeMode) -> Result<Self> {
        let inode = self.inner.read().inode.mknod(name, type_, mode)?;
        Self::new(inode)
    }

    pub fn lookup(&self, name: &str) -> Result<Self> {
        let inode = self.inner.read().inode.lookup(name)?;
        Self::new(inode)
    }

    pub fn link(&self, old: &Vnode, name: &str) -> Result<()> {
        self.inner.read().inode.link(&old.inner.read().inode, name)
    }

    pub fn unlink(&self, name: &str) -> Result<()> {
        self.inner.read().inode.unlink(name)
    }

    pub fn rmdir(&self, name: &str) -> Result<()> {
        self.inner.read().inode.rmdir(name)
    }

    pub fn rename(&self, old_name: &str, target: &Vnode, new_name: &str) -> Result<()> {
        self.inner
            .read()
            .inode
            .rename(old_name, &target.inner.read().inode, new_name)
    }

    pub fn read_link(&self) -> Result<String> {
        self.inner.read().inode.read_link()
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        self.inner.write().inode.write_link(target)
    }

    pub fn readdir(&self, ctx: &mut DirentWriterContext) -> Result<usize> {
        self.inner.read().inode.readdir(ctx)
    }

    pub fn metadata(&self) -> Metadata {
        self.inner.read().inode.metadata()
    }

    pub fn inode_type(&self) -> InodeType {
        self.inner.read().inode.metadata().type_
    }

    pub fn inode_mode(&self) -> InodeMode {
        self.inner.read().inode.metadata().mode
    }

    pub fn len(&self) -> usize {
        self.inner.read().inode.len()
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
}
