// SPDX-License-Identifier: MPL-2.0

#![allow(unused)]

use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
};
use core::ops::Range;

use bitvec::{array::BitArray, prelude::Lsb0};
use ostd::{
    mm::{
        Daddr, DmaDirection, DmaStream, FrameAllocOptions, HasDaddr, Infallible, VmReader,
        VmWriter, PAGE_SIZE,
    },
    sync::{RwLock, SpinLock},
};

/// `DmaPool` is responsible for allocating small streaming DMA segments
/// (equal to or smaller than PAGE_SIZE),
/// referred to as `DmaSegment`.
///
/// A `DmaPool` can only allocate `DmaSegment` of a fixed size.
/// Once a `DmaSegment` is dropped, it will be returned to the pool.
/// If the `DmaPool` is dropped before the associated `DmaSegment`,
/// the `drop` method of the `DmaSegment` will panic.
///
/// Therefore, as a best practice,
/// it is recommended for the `DmaPool` to have a static lifetime.
#[derive(Debug)]
pub struct DmaPool {
    segment_size: usize,
    direction: DmaDirection,
    is_cache_coherent: bool,
    high_watermark: usize,
    avail_pages: SpinLock<VecDeque<Arc<DmaPage>>>,
    all_pages: SpinLock<VecDeque<Arc<DmaPage>>>,
}

impl DmaPool {
    /// Constructs a new `DmaPool` with a specified initial capacity and a high watermark.
    ///
    /// The `DmaPool` starts with `init_size` DMAable pages.
    /// As additional DMA blocks are requested beyond the initial capacity,
    /// the pool dynamically allocates more DMAable pages.
    /// To optimize performance, the pool employs a lazy deallocation strategy:
    /// A DMAable page is freed only if it meets the following conditions:
    /// 1. The page is currently not in use;
    /// 2. The total number of allocated DMAable pages exceeds the specified `high_watermark`.
    ///
    /// The returned pool can be used to allocate small segments for DMA usage.
    /// All allocated segments will have the same DMA direction
    /// and will either all be cache coherent or not cache coherent,
    /// as specified in the parameters.
    pub fn new(
        segment_size: usize,
        init_size: usize,
        high_watermark: usize,
        direction: DmaDirection,
        is_cache_coherent: bool,
    ) -> Arc<Self> {
        assert!(segment_size.is_power_of_two());
        assert!(segment_size >= 64);
        assert!(segment_size <= PAGE_SIZE);
        assert!(high_watermark >= init_size);

        Arc::new_cyclic(|pool| {
            let mut avail_pages = VecDeque::new();
            let mut all_pages = VecDeque::new();

            for _ in 0..init_size {
                let page = Arc::new(
                    DmaPage::new(
                        segment_size,
                        direction,
                        is_cache_coherent,
                        Weak::clone(pool),
                    )
                    .unwrap(),
                );
                avail_pages.push_back(page.clone());
                all_pages.push_back(page);
            }

            Self {
                segment_size,
                direction,
                is_cache_coherent,
                high_watermark,
                avail_pages: SpinLock::new(avail_pages),
                all_pages: SpinLock::new(all_pages),
            }
        })
    }

    /// Allocates a `DmaSegment` from the pool
    pub fn alloc_segment(self: &Arc<Self>) -> Result<DmaSegment, ostd::Error> {
        // Lock order: pool.avail_pages -> pool.all_pages
        //             pool.avail_pages -> page.allocated_segments
        let mut avail_pages = self.avail_pages.disable_irq().lock();
        if avail_pages.is_empty() {
            /// Allocate a new page
            let new_page = {
                let pool = Arc::downgrade(self);
                Arc::new(DmaPage::new(
                    self.segment_size,
                    self.direction,
                    self.is_cache_coherent,
                    pool,
                )?)
            };
            let mut all_pages = self.all_pages.disable_irq().lock();
            avail_pages.push_back(new_page.clone());
            all_pages.push_back(new_page);
        }

        let first_avail_page = avail_pages.front().unwrap();
        let free_segment = first_avail_page.alloc_segment().unwrap();
        if first_avail_page.is_full() {
            avail_pages.pop_front();
        }
        Ok(free_segment)
    }

    /// Returns the number of pages in pool
    fn num_pages(&self) -> usize {
        self.all_pages.disable_irq().lock().len()
    }

    /// Return segment size in pool
    pub fn segment_size(&self) -> usize {
        self.segment_size
    }
}

#[derive(Debug)]
struct DmaPage {
    storage: DmaStream,
    segment_size: usize,
    // `BitArray` is 64 bits, since each `DmaSegment` is bigger than 64 bytes,
    // there's no more than `PAGE_SIZE` / 64 = 64 `DmaSegment`s in a `DmaPage`.
    allocated_segments: SpinLock<BitArray>,
    pool: Weak<DmaPool>,
}

impl DmaPage {
    fn new(
        segment_size: usize,
        direction: DmaDirection,
        is_cache_coherent: bool,
        pool: Weak<DmaPool>,
    ) -> Result<Self, ostd::Error> {
        let dma_stream = {
            let segment = FrameAllocOptions::new(1).alloc_contiguous()?;

            DmaStream::map(segment, direction, is_cache_coherent)
                .map_err(|_| ostd::Error::AccessDenied)?
        };

        Ok(Self {
            storage: dma_stream,
            segment_size,
            allocated_segments: SpinLock::new(BitArray::ZERO),
            pool,
        })
    }

    fn alloc_segment(self: &Arc<Self>) -> Option<DmaSegment> {
        let mut segments = self.allocated_segments.disable_irq().lock();
        let free_segment_index = get_next_free_index(&segments, self.nr_blocks_per_page())?;
        segments.set(free_segment_index, true);

        let segment = DmaSegment {
            size: self.segment_size,
            dma_stream: self.storage.clone(),
            start_addr: self.storage.daddr() + free_segment_index * self.segment_size,
            page: Arc::downgrade(self),
        };

        Some(segment)
    }

    fn is_free(&self) -> bool {
        *self.allocated_segments.lock() == BitArray::<[usize; 1], Lsb0>::ZERO
    }

    const fn nr_blocks_per_page(&self) -> usize {
        PAGE_SIZE / self.segment_size
    }

    fn is_full(&self) -> bool {
        let segments = self.allocated_segments.disable_irq().lock();
        get_next_free_index(&segments, self.nr_blocks_per_page()).is_none()
    }
}

fn get_next_free_index(segments: &BitArray, nr_blocks_per_page: usize) -> Option<usize> {
    let free_segment_index = segments.iter_zeros().next()?;

    if free_segment_index >= nr_blocks_per_page {
        None
    } else {
        Some(free_segment_index)
    }
}

impl HasDaddr for DmaPage {
    fn daddr(&self) -> Daddr {
        self.storage.daddr()
    }
}

/// A small and fixed-size segment of DMA memory.
///
/// The size of `DmaSegment` ranges from 64 bytes to `PAGE_SIZE` and must be 2^K.
/// Each `DmaSegment`'s daddr must be aligned with its size.
#[derive(Debug)]
pub struct DmaSegment {
    dma_stream: DmaStream,
    start_addr: Daddr,
    size: usize,
    page: Weak<DmaPage>,
}

impl HasDaddr for DmaSegment {
    fn daddr(&self) -> Daddr {
        self.start_addr
    }
}

impl DmaSegment {
    pub const fn size(&self) -> usize {
        self.size
    }

    pub fn reader(&self) -> Result<VmReader<'_, Infallible>, ostd::Error> {
        let offset = self.start_addr - self.dma_stream.daddr();
        Ok(self.dma_stream.reader()?.skip(offset).limit(self.size))
    }

    pub fn writer(&self) -> Result<VmWriter<'_, Infallible>, ostd::Error> {
        let offset = self.start_addr - self.dma_stream.daddr();
        Ok(self.dma_stream.writer()?.skip(offset).limit(self.size))
    }

    pub fn sync(&self, byte_range: Range<usize>) -> Result<(), ostd::Error> {
        let offset = self.daddr() - self.dma_stream.daddr();
        let range = byte_range.start + offset..byte_range.end + offset;
        self.dma_stream.sync(range)
    }
}

impl Drop for DmaSegment {
    fn drop(&mut self) {
        let page = self.page.upgrade().unwrap();
        let pool = page.pool.upgrade().unwrap();

        // Keep the same lock order as `pool.alloc_segment`
        // Lock order: pool.avail_pages -> pool.all_pages -> page.allocated_segments
        let mut avail_pages = pool.avail_pages.disable_irq().lock();
        let mut all_pages = pool.all_pages.disable_irq().lock();

        let mut allocated_segments = page.allocated_segments.disable_irq().lock();

        let nr_blocks_per_page = PAGE_SIZE / self.size;
        let became_avail = get_next_free_index(&allocated_segments, nr_blocks_per_page).is_none();

        debug_assert!((page.daddr()..page.daddr() + PAGE_SIZE).contains(&self.daddr()));
        let segment_idx = (self.daddr() - page.daddr()) / self.size;
        allocated_segments.set(segment_idx, false);

        let became_free = allocated_segments.not_any();

        if became_free && all_pages.len() > pool.high_watermark {
            avail_pages.retain(|page_| !Arc::ptr_eq(page_, &page));
            all_pages.retain(|page_| !Arc::ptr_eq(page_, &page));
            return;
        }

        if became_avail {
            avail_pages.push_back(page.clone());
        }
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec::Vec;

    use ostd::prelude::*;

    use super::*;

    #[ktest]
    fn alloc_page_size_segment() {
        let pool = DmaPool::new(PAGE_SIZE, 0, 100, DmaDirection::ToDevice, false);
        let segments1: Vec<_> = (0..100)
            .map(|_| {
                let segment = pool.alloc_segment().unwrap();
                assert_eq!(segment.size(), PAGE_SIZE);
                assert!(segment.reader().is_err());
                assert!(segment.writer().is_ok());
                segment
            })
            .collect();

        assert_eq!(pool.num_pages(), 100);
        drop(segments1);
    }

    #[ktest]
    fn write_to_dma_segment() {
        let pool: Arc<DmaPool> = DmaPool::new(PAGE_SIZE, 1, 2, DmaDirection::ToDevice, false);
        let segment = pool.alloc_segment().unwrap();
        let mut writer = segment.writer().unwrap();
        let data = &[0u8, 1, 2, 3, 4] as &[u8];
        let size = writer.write(&mut VmReader::from(data));
        assert_eq!(size, data.len());
    }

    #[ktest]
    fn free_pool_pages() {
        let pool: Arc<DmaPool> = DmaPool::new(PAGE_SIZE, 10, 50, DmaDirection::ToDevice, false);
        let segments1: Vec<_> = (0..100)
            .map(|_| {
                let segment = pool.alloc_segment().unwrap();
                assert_eq!(segment.size(), PAGE_SIZE);
                assert!(segment.reader().is_err());
                assert!(segment.writer().is_ok());
                segment
            })
            .collect();
        assert_eq!(pool.num_pages(), 100);
        drop(segments1);
        assert_eq!(pool.num_pages(), 50);
    }

    #[ktest]
    fn alloc_small_size_segment() {
        const SEGMENT_SIZE: usize = PAGE_SIZE / 4;
        let pool: Arc<DmaPool> =
            DmaPool::new(SEGMENT_SIZE, 0, 10, DmaDirection::Bidirectional, false);
        let segments1: Vec<_> = (0..100)
            .map(|_| {
                let segment = pool.alloc_segment().unwrap();
                assert_eq!(segment.size(), PAGE_SIZE / 4);
                assert!(segment.reader().is_ok());
                assert!(segment.writer().is_ok());
                segment
            })
            .collect();

        assert_eq!(pool.num_pages(), 100 / 4);
        drop(segments1);
        assert_eq!(pool.num_pages(), 10);
    }

    #[ktest]
    fn read_dma_segments() {
        const SEGMENT_SIZE: usize = PAGE_SIZE / 4;
        let pool: Arc<DmaPool> =
            DmaPool::new(SEGMENT_SIZE, 1, 2, DmaDirection::Bidirectional, false);
        let segment = pool.alloc_segment().unwrap();
        assert_eq!(pool.num_pages(), 1);
        let mut writer = segment.writer().unwrap();
        let data = &[0u8, 1, 2, 3, 4] as &[u8];
        let size = writer.write(&mut VmReader::from(data));
        assert_eq!(size, data.len());

        let mut read_buf = [0u8; 5];
        let mut reader = segment.reader().unwrap();
        reader.read(&mut VmWriter::from(&mut read_buf as &mut [u8]));
        assert_eq!(&read_buf, data);
    }
}
