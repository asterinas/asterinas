// SPDX-License-Identifier: MPL-2.0

#![allow(unused)]

use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
};
use core::ops::Range;

use aster_frame::{
    config::PAGE_SIZE,
    sync::{RwLock, SpinLock},
    vm::{Daddr, DmaDirection, DmaStream, HasDaddr, VmAllocOptions, VmReader, VmWriter},
};
use bitvec::{array::BitArray, prelude::Lsb0};
use ktest::ktest;

/// `DmaPool` is responsible for allocating small streaming DMA areas (equal to or smaller than PAGE_SIZE),
/// referred to as `DmaBlock`.
///
/// A `DmaPool` can only allocate `DmaBlock` of a fixed size. Once a `DmaBlock` is dropped, it will be returned
/// to the pool. If the `DmaPool` is dropped before the associated `DmaBlock`, the `drop` method of the `DmaBlock`
/// will panic. Therefore, as a best practice, it is recommended for the `DmaPool` to have a static lifetime.
#[derive(Debug)]
pub struct DmaPool {
    direction: DmaDirection,
    block_size: usize,
    is_cache_coherent: bool,
    pages: RwLock<VecDeque<Arc<DmaPage>>>,
}

#[derive(Debug)]
struct DmaPage {
    storage: DmaStream,
    // `BitArray` is 64 bits, since each `DmaBlock` is bigger than 64Bytes, there's no more
    // than `PAGE_SIZE` / 64 = 64 `DmaBlock`s in a `DmaPage`.
    allocated_blocks: SpinLock<BitArray>,
}

impl DmaPage {
    fn alloc_block(self: &Arc<Self>, block_size: usize) -> Option<DmaBlock> {
        let mut blocks = self.allocated_blocks.lock_irq_disabled();

        let free_block_index = blocks.iter_zeros().nth(0)?;
        if free_block_index * block_size >= self.storage.nbytes() {
            return None;
        }
        blocks.set(free_block_index, true);

        let block = DmaBlock {
            dma_stream: self.storage.clone(),
            start_addr: self.storage.daddr() + free_block_index * block_size,
            size: block_size,
            page: Arc::downgrade(self),
        };

        Some(block)
    }

    fn is_free(&self) -> bool {
        *self.allocated_blocks.lock() == BitArray::<[usize; 1], Lsb0>::ZERO
    }
}

impl HasDaddr for DmaPage {
    fn daddr(&self) -> Daddr {
        self.storage.daddr()
    }
}

/// Small streaming DMA areas. The size of `DmaBlock` ranges from 64 bytes to `PAGE_SIZE` and must be 2^K.
#[derive(Debug)]
pub struct DmaBlock {
    dma_stream: DmaStream,
    start_addr: Daddr,
    size: usize,
    page: Weak<DmaPage>,
}

impl HasDaddr for DmaBlock {
    fn daddr(&self) -> Daddr {
        self.start_addr
    }
}

impl DmaBlock {
    pub const fn size(&self) -> usize {
        self.size
    }

    pub fn reader(&self) -> Result<VmReader<'_>, aster_frame::Error> {
        let offset = self.start_addr - self.dma_stream.daddr();
        Ok(self.dma_stream.reader()?.skip(offset).limit(self.size))
    }

    pub fn writer(&self) -> Result<VmWriter<'_>, aster_frame::Error> {
        let offset = self.start_addr - self.dma_stream.daddr();
        Ok(self.dma_stream.writer()?.skip(offset).limit(self.size))
    }

    pub fn sync(&self, byte_range: Range<usize>) -> Result<(), aster_frame::Error> {
        let offset = self.daddr() - self.dma_stream.daddr();
        let range = byte_range.start + offset..byte_range.end + offset;
        self.dma_stream.sync(range)
    }
}

impl Drop for DmaBlock {
    fn drop(&mut self) {
        let page = self.page.upgrade().unwrap();
        debug_assert!((page.daddr()..page.daddr() + PAGE_SIZE).contains(&self.daddr()));
        let block_idx = (self.daddr() - page.daddr()) / self.size;
        page.allocated_blocks.lock().set(block_idx, false);
    }
}

impl DmaPool {
    pub fn new(block_size: usize, direction: DmaDirection, is_cache_coherent: bool) -> Self {
        assert!(block_size.is_power_of_two());
        assert!(block_size >= 64);
        assert!(block_size <= PAGE_SIZE);
        Self {
            direction,
            block_size,
            is_cache_coherent,
            pages: RwLock::new(VecDeque::new()),
        }
    }

    pub fn alloc_block(&self) -> Result<DmaBlock, aster_frame::Error> {
        for page in self.pages.read_irq_disabled().iter() {
            if let Some(block) = page.alloc_block(self.block_size) {
                return Ok(block);
            }
        }

        let dma_stream = {
            let vm_segment = {
                let mut options = VmAllocOptions::new(1);
                options.is_contiguous(true);
                options.alloc_contiguous()?
            };

            DmaStream::map(vm_segment, self.direction, self.is_cache_coherent)
                .map_err(|_| aster_frame::Error::AccessDenied)?
        };

        let dma_page = Arc::new(DmaPage {
            storage: dma_stream,
            allocated_blocks: SpinLock::new(BitArray::ZERO),
        });

        let block = dma_page.alloc_block(self.block_size).unwrap();
        self.pages.write_irq_disabled().push_back(dma_page);
        Ok(block)
    }

    /// Free pages that are not used in the pool
    pub fn free_pages(&self) {
        self.pages
            .write_irq_disabled()
            .retain(|page| !page.is_free())
    }

    pub fn num_pages(&self) -> usize {
        self.pages.read_irq_disabled().len()
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec::Vec;

    use super::*;

    #[ktest]
    fn alloc_page_size_block() {
        let pool = DmaPool::new(PAGE_SIZE, DmaDirection::ToDevice, false);
        let blocks1: Vec<_> = (0..100)
            .map(|_| {
                let block = pool.alloc_block().unwrap();
                assert_eq!(block.size(), PAGE_SIZE);
                assert!(block.reader().is_err());
                assert!(block.writer().is_ok());
                block
            })
            .collect();

        assert_eq!(pool.num_pages(), 100);
    }

    #[ktest]
    fn write_to_dma_block() {
        let pool = DmaPool::new(PAGE_SIZE, DmaDirection::ToDevice, false);
        let block = pool.alloc_block().unwrap();
        let mut writer = block.writer().unwrap();
        let data = &[0u8, 1, 2, 3, 4] as &[u8];
        let size = writer.write(&mut VmReader::from(data));
        assert_eq!(size, data.len());
    }

    #[ktest]
    fn free_pool_pages() {
        let pool = DmaPool::new(PAGE_SIZE, DmaDirection::ToDevice, false);
        let blocks1: Vec<_> = (0..100)
            .map(|_| {
                let block = pool.alloc_block().unwrap();
                assert_eq!(block.size(), PAGE_SIZE);
                assert!(block.reader().is_err());
                assert!(block.writer().is_ok());
                block
            })
            .collect();
        pool.free_pages();
        assert_eq!(pool.num_pages(), 100);
        drop(blocks1);
        pool.free_pages();
        assert_eq!(pool.num_pages(), 0);
    }

    #[ktest]
    fn alloc_samll_size_block() {
        let pool = DmaPool::new(PAGE_SIZE / 4, DmaDirection::Bidirectional, false);
        let blocks1: Vec<_> = (0..100)
            .map(|_| {
                let block = pool.alloc_block().unwrap();
                assert_eq!(block.size(), PAGE_SIZE / 4);
                assert!(block.reader().is_ok());
                assert!(block.writer().is_ok());
                block
            })
            .collect();

        assert_eq!(pool.num_pages(), 100 / 4);
        drop(blocks1);
        assert_eq!(pool.num_pages(), 100 / 4);
    }

    #[ktest]
    fn read_dma_blocks() {
        let pool = DmaPool::new(PAGE_SIZE / 4, DmaDirection::Bidirectional, false);
        let block = pool.alloc_block().unwrap();
        assert_eq!(pool.num_pages(), 1);
        let mut writer = block.writer().unwrap();
        let data = &[0u8, 1, 2, 3, 4] as &[u8];
        let size = writer.write(&mut VmReader::from(data));
        assert_eq!(size, data.len());

        let mut read_buf = [0u8; 5];
        let mut reader = block.reader().unwrap();
        reader.read(&mut VmWriter::from(&mut read_buf as &mut [u8]));
        assert_eq!(&read_buf, data);
    }
}
