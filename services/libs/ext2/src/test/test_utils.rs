use crate::inode::{Ext2Inode, FilePerm, FileType};
use crate::prelude::*;
use crate::traits::PageIoObj;

use core::cmp::Ordering;
use mem_storage::{MemArea, MemStorage, MemStorageIterator};

pub struct PageCacheTest {
    inner: RwLock<(Vec<BlockBuf>, usize)>,
    weak_inode: Weak<dyn PageIoObj>,
}

impl PageCacheTest {
    pub fn new(size: usize, inode: Weak<dyn PageIoObj>) -> Result<Self> {
        let blocks_count = size.div_ceil(BLOCK_SIZE) as usize;
        let cache = Vec::with_capacity(blocks_count);
        Ok(Self {
            inner: RwLock::new((cache, blocks_count)),
            weak_inode: inode,
        })
    }

    fn ensure_all_pages_exist(&self) -> Result<()> {
        let mut inner = self.inner.write();

        if inner.0.len() < inner.1 {
            for idx in inner.0.len()..inner.1 {
                let boxed_slice = vec![0u8; BLOCK_SIZE].into_boxed_slice();
                let mut block = BlockBuf::from_boxed_slice(boxed_slice);
                self.weak_inode
                    .upgrade()
                    .unwrap()
                    .read_page(idx, &mut block)?;
                inner.0.push(block);
            }
        }

        Ok(())
    }
}

impl Debug for PageCacheTest {
    fn fmt(&self, f: &mut core::fmt::Formatter) -> core::fmt::Result {
        f.debug_struct("PageCacheTest")
            .field("size", &self.inner.write().1)
            .finish()
    }
}

impl PageCache for PageCacheTest {
    fn new(size: usize, inode: Weak<dyn PageIoObj>) -> Box<dyn PageCache> {
        Box::new(Self::new(size, inode).unwrap())
    }

    fn resize(&self, new_len: usize) -> Result<()> {
        self.ensure_all_pages_exist()?;

        let mut inner = self.inner.write();
        let old_blocks = inner.0.len();
        let new_blocks = new_len.div_ceil(BLOCK_SIZE);

        match new_blocks.cmp(&old_blocks) {
            Ordering::Greater => {
                for _ in old_blocks..new_blocks {
                    let boxed_slice = vec![0u8; BLOCK_SIZE].into_boxed_slice();
                    let block = BlockBuf::from_boxed_slice(boxed_slice);
                    inner.0.push(block);
                }
                inner.1 = new_blocks;
            }
            Ordering::Equal => (),
            Ordering::Less => {
                inner.0.truncate(new_blocks);
                inner.1 = new_blocks;
            }
        }

        Ok(())
    }

    fn pages(&self) -> Box<dyn MemStorage> {
        self.ensure_all_pages_exist().unwrap();
        Box::new(self.inner.read().0.clone())
    }

    /// Evict the data within a specified range from the page cache and persist
    /// them to the disk.
    fn evict_range(&self, range: Range<usize>) -> Result<()> {
        self.ensure_all_pages_exist()?;
        let page_idx_range = (range.start.div_floor(BLOCK_SIZE))..(range.end.div_ceil(BLOCK_SIZE));
        for idx in page_idx_range {
            if let Some(page) = self.inner.read().0.get(idx) {
                self.weak_inode.upgrade().unwrap().write_page(idx, page)?
            }
        }
        Ok(())
    }
}

#[derive(Debug, Clone)]
pub struct BlockBuf {
    inner: Arc<Box<[u8]>>,
}

impl BlockBuf {
    pub fn from_boxed_slice(boxed_slice: Box<[u8]>) -> Self {
        debug_assert!(boxed_slice.len() == BLOCK_SIZE);
        Self {
            inner: Arc::new(boxed_slice),
        }
    }

    pub fn as_slice(&self) -> &[u8] {
        self.inner.as_ref().as_ref()
    }

    pub fn as_mut_slice(&self) -> &mut [u8] {
        // Safety: The BlockBuf can be read or write simultaneously.
        unsafe {
            core::slice::from_raw_parts_mut(self.inner.as_ref().as_ptr() as *mut u8, BLOCK_SIZE)
        }
    }

    pub fn len(&self) -> usize {
        self.as_slice().len()
    }

    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl MemStorage for BlockBuf {
    fn mem_areas(&self, is_writable: bool) -> mem_storage::Result<MemStorageIterator> {
        let mem_area = if is_writable {
            MemArea::from_slice_mut(self.as_mut_slice())
        } else {
            MemArea::from_slice(self.as_slice())
        };

        Ok(MemStorageIterator::from_vec(vec![mem_area]))
    }

    fn total_len(&self) -> usize {
        self.len()
    }
}

#[derive(Debug, Clone)]
pub struct VnodeTest {
    inode: Arc<Ext2Inode>,
}

impl VnodeTest {
    pub fn new(inode: Arc<Ext2Inode>) -> Self {
        Self { inode }
    }

    pub fn create(&self, name: &str, file_type: FileType, file_perm: FilePerm) -> Result<Self> {
        let inode = self
            .inode
            .create::<PageCacheTest>(name, file_type, file_perm)?;
        Ok(Self { inode })
    }

    pub fn lookup(&self, name: &str) -> Result<Self> {
        let inode = self.inode.lookup::<PageCacheTest>(name)?;
        Ok(Self { inode })
    }

    pub fn write_link(&self, target: &str) -> Result<()> {
        self.inode.write_link(target)
    }

    pub fn read_link(&self) -> Result<String> {
        self.inode.read_link()
    }

    pub fn read_at(&self, offset: usize, buf: &mut [u8]) -> Result<usize> {
        self.inode.read_at(offset, buf)
    }

    pub fn write_at(&self, offset: usize, buf: &[u8]) -> Result<usize> {
        self.inode.write_at(offset, buf)
    }
}

impl Drop for VnodeTest {
    fn drop(&mut self) {
        if Arc::strong_count(&self.inode) == 1 {
            self.inode.sync_data().unwrap();
        }
    }
}
