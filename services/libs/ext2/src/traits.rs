use crate::prelude::*;

/// PageCache
pub trait PageCache: Sync + Send + Debug {
    fn new(size: usize, page_io_obj: Weak<dyn PageIoObj>) -> Box<dyn PageCache>
    where
        Self: Sized;

    fn resize(&self, new_size: usize) -> Result<()>;

    fn pages(&self) -> Box<dyn MemStorage>;

    /// Evict the data within a specified range from the page cache and persist
    /// them to the disk.
    fn evict_range(&self, range: Range<usize>) -> Result<()>;
}

/// Page granularity I/O object
pub trait PageIoObj: Sync + Send {
    fn read_page(&self, idx: usize, page: &dyn MemStorage) -> Result<()>;

    fn write_page(&self, idx: usize, page: &dyn MemStorage) -> Result<()>;

    fn len(&self) -> usize;
}

impl PageIoObj for crate::inode::Ext2Inode {
    fn read_page(&self, idx: usize, page: &dyn MemStorage) -> Result<()> {
        self.read_block(BlockId::new(idx as u32), page)
    }

    fn write_page(&self, idx: usize, page: &dyn MemStorage) -> Result<()> {
        self.write_block(BlockId::new(idx as u32), page)
    }

    fn len(&self) -> usize {
        self.file_size() as _
    }
}
