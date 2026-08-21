// SPDX-License-Identifier: MPL-2.0

use alloc::{
    collections::VecDeque,
    sync::{Arc, Weak},
};
use core::ops::Range;

use aster_bigtcp::packet::{DmaHandle, RxDmaAllocator, RxDmaHandle, TxDmaAllocator, TxDmaHandle};
use aster_softirq::BottomHalfDisabled;
use bitvec::array::BitArray;
use ostd::{
    Error,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, Infallible, PAGE_SIZE, USegment, VmReader, VmWriter,
        dma::{DmaDirection, DmaStream, FromDevice, ToDevice},
        io::util::HasVmReaderWriter,
    },
    sync::SpinLock,
};

/// `DmaPool` is responsible for allocating small streaming DMA segments
/// (equal to or smaller than `PAGE_SIZE`),
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
pub struct DmaPool<D: DmaDirection> {
    segment_size: usize,
    is_cache_coherent: bool,
    high_watermark: usize,
    avail_pages: SpinLock<VecDeque<Arc<DmaPage<D>>>, BottomHalfDisabled>,
    all_pages: SpinLock<VecDeque<Arc<DmaPage<D>>>, BottomHalfDisabled>,
}

impl<D: DmaDirection> DmaPool<D> {
    /// Constructs a new `DmaPool` with a specified initial capacity and a high watermark.
    ///
    /// The `DmaPool` starts with `init_size` DMAable pages.
    /// As additional DMA blocks are requested beyond the initial capacity,
    /// the pool dynamically allocates more DMAable pages.
    /// To optimize performance, the pool employs a lazy deallocation strategy:
    /// A DMAable page is freed only if it meets the following conditions:
    /// 1. The page is currently not in use;
    /// 2. The total number of available DMAable pages (i.e., allocated pages that have at
    ///    least one free segment) exceeds the specified `high_watermark`.
    ///
    /// The returned pool can be used to allocate small segments for DMA usage.
    /// All allocated segments will have the same DMA direction
    /// and will either all be cache coherent or not cache coherent,
    /// as specified in the parameters.
    pub fn new(
        segment_size: usize,
        init_size: usize,
        high_watermark: usize,
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
                    DmaPage::new(segment_size, is_cache_coherent, Weak::clone(pool)).unwrap(),
                );
                avail_pages.push_back(page.clone());
                all_pages.push_back(page);
            }

            Self {
                segment_size,
                is_cache_coherent,
                high_watermark,
                avail_pages: SpinLock::new(avail_pages),
                all_pages: SpinLock::new(all_pages),
            }
        })
    }

    /// Allocates a segment from the pool.
    pub fn alloc_segment(self: &Arc<Self>) -> Result<DmaSegment<D>, Error> {
        let (page, start_offset) = self.alloc_page_segment()?;
        Ok(DmaSegment { page, start_offset })
    }

    fn alloc_page_segment(self: &Arc<Self>) -> Result<(Arc<DmaPage<D>>, usize), Error> {
        // Lock order: pool.avail_pages -> pool.all_pages
        //             pool.avail_pages -> page.allocated_segments
        let mut avail_pages = self.avail_pages.lock();
        if avail_pages.is_empty() {
            // Allocate a new page
            let new_page = {
                let pool = Arc::downgrade(self);
                Arc::new(DmaPage::new(
                    self.segment_size,
                    self.is_cache_coherent,
                    pool,
                )?)
            };
            let mut all_pages = self.all_pages.lock();
            avail_pages.push_back(new_page.clone());
            all_pages.push_back(new_page);
        }

        let first_avail_page = avail_pages.front().unwrap();
        let start_offset = first_avail_page.alloc_segment().unwrap();
        let page = first_avail_page.clone();
        if first_avail_page.is_full() {
            avail_pages.pop_front();
        }
        Ok((page, start_offset))
    }

    /// Returns the number of pages in the pool.
    #[cfg(ktest)]
    fn num_pages(&self) -> usize {
        self.all_pages.lock().len()
    }

    /// Returns the segment size of the pool.
    pub fn segment_size(&self) -> usize {
        self.segment_size
    }
}

#[derive(Debug)]
struct DmaPage<D: DmaDirection> {
    segment: USegment,
    storage: DmaStream<D>,
    segment_size: usize,
    // A `BitArray` has 64 bits. Since each `DmaSegment` is bigger than 64 bytes,
    // there are no more than `PAGE_SIZE` / 64 = 64 `DmaSegment`s in a `DmaPage`.
    allocated_segments: SpinLock<BitArray, BottomHalfDisabled>,
    pool: Weak<DmaPool<D>>,
}

impl<D: DmaDirection> DmaPage<D> {
    fn new(
        segment_size: usize,
        is_cache_coherent: bool,
        pool: Weak<DmaPool<D>>,
    ) -> Result<Self, Error> {
        let segment: USegment = FrameAllocOptions::new().alloc_segment(1)?.into();
        let dma_stream = DmaStream::<D>::map(segment.clone(), is_cache_coherent)?;

        Ok(Self {
            segment,
            storage: dma_stream,
            segment_size,
            allocated_segments: SpinLock::new(BitArray::ZERO),
            pool,
        })
    }

    fn alloc_segment(&self) -> Option<usize> {
        let mut segments = self.allocated_segments.lock();
        let free_segment_index = get_next_free_index(&segments, self.nr_blocks_per_page())?;
        segments.set(free_segment_index, true);

        Some(free_segment_index * self.segment_size)
    }

    const fn nr_blocks_per_page(&self) -> usize {
        PAGE_SIZE / self.segment_size
    }

    fn is_full(&self) -> bool {
        let segments = self.allocated_segments.lock();
        get_next_free_index(&segments, self.nr_blocks_per_page()).is_none()
    }

    fn release_segment(self: &Arc<Self>, start_offset: usize) {
        let pool = self.pool.upgrade().unwrap();

        // Keep the same lock order as `pool.alloc_segment`
        // Lock order: pool.avail_pages -> pool.all_pages
        //             pool.avail_pages -> page.allocated_segments
        let mut avail_pages = pool.avail_pages.lock();

        let (became_avail, became_free) = {
            let mut allocated_segments = self.allocated_segments.lock();

            let nr_blocks_per_page = PAGE_SIZE / self.segment_size;
            let became_avail =
                get_next_free_index(&allocated_segments, nr_blocks_per_page).is_none();

            debug_assert!(start_offset < PAGE_SIZE);
            debug_assert_eq!(start_offset % self.segment_size, 0);
            let segment_idx = start_offset / self.segment_size;
            debug_assert!(allocated_segments[segment_idx]);
            allocated_segments.set(segment_idx, false);

            let became_free = allocated_segments.not_any();
            (became_avail, became_free)
        };

        if became_free && avail_pages.len() > pool.high_watermark {
            let mut all_pages = pool.all_pages.lock();
            avail_pages.retain(|page| !Arc::ptr_eq(page, self));
            all_pages.retain(|page| !Arc::ptr_eq(page, self));
            return;
        }

        if became_avail {
            avail_pages.push_back(self.clone());
        }
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

impl<D: DmaDirection> HasDaddr for DmaPage<D> {
    fn daddr(&self) -> Daddr {
        self.storage.daddr()
    }
}

/// A small and fixed-size segment of DMA memory.
///
/// The size of a `DmaSegment` ranges from 64 bytes to `PAGE_SIZE`
/// and is a power of two.
/// Each `DmaSegment`'s DMA address is guaranteed to be aligned with its size.
#[derive(Debug)]
pub struct DmaSegment<D: DmaDirection> {
    page: Arc<DmaPage<D>>,
    start_offset: usize,
}

impl<D: DmaDirection> HasDaddr for DmaSegment<D> {
    fn daddr(&self) -> Daddr {
        self.page.daddr() + self.start_offset
    }
}

impl<D: DmaDirection> DmaSegment<D> {
    pub fn size(&self) -> usize {
        self.page.segment_size
    }

    pub fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        let mut reader = self.page.storage.reader()?;
        reader.skip(self.start_offset).limit(self.page.segment_size);
        Ok(reader)
    }

    pub fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        let mut writer = self.page.storage.writer()?;
        writer.skip(self.start_offset).limit(self.page.segment_size);
        Ok(writer)
    }

    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        if byte_range.start > self.page.segment_size || byte_range.end > self.page.segment_size {
            return Err(Error::InvalidArgs);
        }
        let range = byte_range.start + self.start_offset..byte_range.end + self.start_offset;
        self.page.storage.sync_from_device(range)
    }

    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        if byte_range.start > self.page.segment_size || byte_range.end > self.page.segment_size {
            return Err(Error::InvalidArgs);
        }
        let range = byte_range.start + self.start_offset..byte_range.end + self.start_offset;
        self.page.storage.sync_to_device(range)
    }
}

impl<D: DmaDirection> Drop for DmaSegment<D> {
    fn drop(&mut self) {
        self.page.release_segment(self.start_offset);
    }
}

impl DmaHandle for DmaPage<ToDevice> {
    fn on_drop(self: Arc<Self>, alloc_start: usize) {
        self.release_segment(alloc_start);
    }
}

impl TxDmaHandle for DmaPage<ToDevice> {
    fn sync_to_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        self.storage.sync_to_device(byte_range)
    }
}

impl TxDmaAllocator for DmaPool<ToDevice> {
    fn alloc(
        self: &Arc<Self>,
        nbytes: usize,
    ) -> Result<(USegment, Arc<dyn TxDmaHandle>, usize), Error> {
        if nbytes > self.segment_size {
            return Err(Error::InvalidArgs);
        }

        let (page, alloc_start) = self.alloc_page_segment()?;
        let segment = page.segment.clone();
        let handle: Arc<dyn TxDmaHandle> = page;
        Ok((segment, handle, alloc_start))
    }
}

impl DmaHandle for DmaPage<FromDevice> {
    fn on_drop(self: Arc<Self>, alloc_start: usize) {
        self.release_segment(alloc_start);
    }
}

impl RxDmaHandle for DmaPage<FromDevice> {
    fn sync_from_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        self.storage.sync_from_device(byte_range)
    }
}

impl RxDmaAllocator for DmaPool<FromDevice> {
    fn alloc(
        self: &Arc<Self>,
        nbytes: usize,
    ) -> Result<(USegment, Arc<dyn RxDmaHandle>, usize), Error> {
        if nbytes > self.segment_size {
            return Err(Error::InvalidArgs);
        }

        let (page, alloc_start) = self.alloc_page_segment()?;
        let segment = page.segment.clone();
        let handle: Arc<dyn RxDmaHandle> = page;
        Ok((segment, handle, alloc_start))
    }
}

#[cfg(ktest)]
mod test {
    use alloc::vec::Vec;

    use ostd::{mm::dma::FromAndToDevice, prelude::*};

    use super::*;

    #[ktest]
    fn alloc_page_size_segment() {
        let pool = DmaPool::<ToDevice>::new(PAGE_SIZE, 0, 100, false);
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
        let pool: Arc<DmaPool<ToDevice>> = DmaPool::new(PAGE_SIZE, 1, 2, false);
        let segment = pool.alloc_segment().unwrap();
        let mut writer = segment.writer().unwrap();
        let data = &[0u8, 1, 2, 3, 4] as &[u8];
        let size = writer.write(&mut VmReader::from(data));
        assert_eq!(size, data.len());
    }

    #[ktest]
    fn free_pool_pages() {
        let pool: Arc<DmaPool<ToDevice>> = DmaPool::new(PAGE_SIZE, 10, 50, false);
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
        assert_eq!(pool.num_pages(), 51);
    }

    #[ktest]
    fn alloc_small_size_segment() {
        const SEGMENT_SIZE: usize = PAGE_SIZE / 4;
        let pool: Arc<DmaPool<FromAndToDevice>> = DmaPool::new(SEGMENT_SIZE, 0, 10, false);
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
        let pool: Arc<DmaPool<FromAndToDevice>> = DmaPool::new(SEGMENT_SIZE, 1, 2, false);
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
