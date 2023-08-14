use crate::inode::{Ext2Inode, FilePerm, FileType, FAST_SYMLINK_MAX_LEN};
use crate::prelude::*;

use core::cmp::Ordering;

pub struct Vnode<'a> {
    inode: Arc<Ext2Inode>,
    cache: PageCache<'a>,
}

impl<'a> Vnode<'a> {
    pub fn new(inode: Arc<Ext2Inode>) -> Result<Self> {
        let cache = PageCache::new(&inode)?;

        Ok(Self { inode, cache })
    }

    pub fn create(&self, name: &str, file_type: FileType, file_perm: FilePerm) -> Result<Self> {
        let new_inode =
            self.inode
                .create(name, file_type, file_perm, self.cache.inner.write().deref())?;
        Self::new(new_inode)
    }

    pub fn lookup(&self, name: &str) -> Result<Self> {
        let inode = self.inode.lookup(name, self.cache.inner.read().deref())?;
        Self::new(inode)
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        let file_size = self.inode.file_size() as usize;
        let new_size = offset + buf.len();
        if new_size > file_size {
            self.inode.resize(new_size)?;
            self.cache.resize(self.inode.blocks_count() as usize)?;
            self.cache.write_bytes_at(offset, buf)?;
        }

        Ok(buf.len())
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        let (offset, read_len) = {
            let file_size = self.inode.file_size() as usize;
            let start = file_size.min(offset);
            let end = file_size.min(offset + buf.len());
            (start, end - start)
        };
        self.cache.read_bytes_at(offset, &mut buf[..read_len])?;
        Ok(read_len)
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        self.inode.resize(target.len())?;
        self.cache.resize(self.inode.blocks_count() as usize)?;
        self.inode
            .write_link(target, self.cache.inner.read().deref())?;
        Ok(())
    }

    pub fn read_link(&self) -> Result<String> {
        self.inode.read_link(self.cache.inner.read().deref())
    }
}

impl<'a> Drop for Vnode<'a> {
    fn drop(&mut self) {
        for (idx, block) in self.cache.inner.read().iter().enumerate() {
            self.inode
                .write_block(BlockId::new(idx as u32), block)
                .unwrap();
        }
    }
}

impl<'a> Debug for Vnode<'a> {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("Vnode")
            .field("inode", &self.inode)
            .field("page_cache_size", &self.cache.inner.read().len())
            .finish()
    }
}

struct PageCache<'a> {
    inner: RwLock<Vec<BioBuf<'a>>>,
}

impl<'a> PageCache<'a> {
    pub fn new(inode: &Arc<Ext2Inode>) -> Result<Self> {
        let blocks_count = inode.blocks_count() as usize;
        let mut cache = Vec::with_capacity(blocks_count);
        for idx in 0..blocks_count {
            let boxed_slice = vec![0u8; BLOCK_SIZE].into_boxed_slice();
            let mut block = BioBuf::from_boxed_slice(boxed_slice);
            inode.read_block(BlockId::new(idx as u32), &mut block)?;
            cache.push(block);
        }
        Ok(Self {
            inner: RwLock::new(cache),
        })
    }

    pub fn resize(&self, blocks: usize) -> Result<()> {
        let mut inner = self.inner.write();
        let old_blocks = inner.len();

        match blocks.cmp(&old_blocks) {
            Ordering::Greater => {
                for _ in old_blocks..blocks {
                    let boxed_slice = vec![0u8; BLOCK_SIZE].into_boxed_slice();
                    let block = BioBuf::from_boxed_slice(boxed_slice);
                    inner.push(block);
                }
            }
            Ordering::Equal => (),
            Ordering::Less => {
                inner.truncate(blocks);
            }
        }

        Ok(())
    }
}

impl<'a> GenericIo for PageCache<'a> {
    fn read_bytes_at(&self, offset: usize, buf: &mut [u8]) -> mem_storage::Result<()> {
        let inner = self.inner.read();
        inner.read_bytes_at(offset, buf)
    }

    fn write_bytes_at(&self, offset: usize, buf: &[u8]) -> mem_storage::Result<()> {
        let inner = self.inner.write();
        inner.write_bytes_at(offset, buf)
    }
}
