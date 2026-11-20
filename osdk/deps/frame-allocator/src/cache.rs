// SPDX-License-Identifier: MPL-2.0

//! A fixed-size local cache for frame allocation.

use core::{alloc::Layout, cell::RefCell};

use ostd::{
    cpu_local,
    irq::DisabledLocalIrqGuard,
    mm::{PAGE_SIZE, Paddr},
};

cpu_local! {
    static CACHE: RefCell<CacheOfSizes> = RefCell::new(CacheOfSizes::new());
}

struct CacheOfSizes {
    cache1: CacheArray<1, 12>,
    cache2: CacheArray<2, 6>,
    cache3: CacheArray<3, 6>,
    cache4: CacheArray<4, 6>,
}

/// A fixed-size local cache for frame allocation.
///
/// Each cache array contains at most `COUNT` segments. Each segment contains
/// `NR_CONT_FRAMES` contiguous frames.
struct CacheArray<const NR_CONT_FRAMES: usize, const COUNT: usize> {
    inner: [Paddr; COUNT],
    size: usize,
}

impl<const NR_CONT_FRAMES: usize, const COUNT: usize> CacheArray<NR_CONT_FRAMES, COUNT> {
    const fn new() -> Self {
        Self {
            inner: [0; COUNT],
            size: 0,
        }
    }

    /// The size of the segments that this cache manages.
    const fn segment_size() -> usize {
        NR_CONT_FRAMES * PAGE_SIZE
    }

    /// Allocates a segment of frames.
    ///
    /// If the cache is empty, it tries a batch refill before falling back to
    /// allocating only the requested segment.
    fn alloc(&mut self, guard: &DisabledLocalIrqGuard) -> Option<Paddr> {
        self.alloc_with(|layout| super::pools::alloc(guard, layout))
    }

    fn alloc_with(
        &mut self,
        mut allocate_fn: impl FnMut(Layout) -> Option<Paddr>,
    ) -> Option<Paddr> {
        if let Some(frame) = self.pop_front() {
            return Some(frame);
        }

        let nr_to_alloc = COUNT * 2 / 3;
        let Some(allocated) = allocate_fn(
            Layout::from_size_align(nr_to_alloc * Self::segment_size(), PAGE_SIZE).unwrap(),
        ) else {
            // Fragmentation can prevent a refill even when one segment fits.
            return allocate_fn(Layout::from_size_align(Self::segment_size(), PAGE_SIZE).unwrap());
        };

        // `rev()` keeps future allocations contiguous from lower to higher
        // addresses. This increases the likelihood that they can be merged,
        // for example, when performing device operations.
        for i in (1..nr_to_alloc).rev() {
            self.push_front(allocated + i * Self::segment_size());
        }

        Some(allocated)
    }

    /// Deallocates a segment of frames.
    ///
    /// It may deallocate directly to this cache. If the cache is full, it will
    /// deallocate to the global pool.
    fn dealloc(&mut self, guard: &DisabledLocalIrqGuard, addr: Paddr) {
        if self.push_front(addr).is_none() {
            let nr_to_dealloc = COUNT * 2 / 3 + 1;

            let segments = (0..nr_to_dealloc).map(|i| {
                if i == 0 {
                    (addr, Self::segment_size())
                } else {
                    (self.pop_front().unwrap(), Self::segment_size())
                }
            });

            super::pools::dealloc(guard, segments);
        };
    }

    fn push_front(&mut self, frame: Paddr) -> Option<()> {
        if self.size == COUNT {
            return None;
        }

        self.inner[self.size] = frame;
        self.size += 1;
        Some(())
    }

    fn pop_front(&mut self) -> Option<Paddr> {
        if self.size == 0 {
            return None;
        }

        let frame = self.inner[self.size - 1];
        self.size -= 1;
        Some(frame)
    }
}

impl CacheOfSizes {
    const fn new() -> Self {
        Self {
            cache1: CacheArray::new(),
            cache2: CacheArray::new(),
            cache3: CacheArray::new(),
            cache4: CacheArray::new(),
        }
    }
}

pub(super) fn alloc(guard: &DisabledLocalIrqGuard, layout: Layout) -> Option<Paddr> {
    let nr_frames = layout.size() / PAGE_SIZE;
    if layout.align() > PAGE_SIZE {
        return super::pools::alloc(guard, layout);
    }

    let cache_cell = CACHE.get_with(guard);
    let mut cache = cache_cell.borrow_mut();

    match nr_frames {
        1 => cache.cache1.alloc(guard),
        2 => cache.cache2.alloc(guard),
        3 => cache.cache3.alloc(guard),
        4 => cache.cache4.alloc(guard),
        _ => super::pools::alloc(guard, layout),
    }
}

pub(super) fn dealloc(guard: &DisabledLocalIrqGuard, addr: Paddr, size: usize) {
    let nr_frames = size / PAGE_SIZE;
    if nr_frames > 4 {
        super::pools::dealloc(guard, [(addr, size)].into_iter());
        return;
    }

    let cache_cell = CACHE.get_with(guard);
    let mut cache = cache_cell.borrow_mut();

    match nr_frames {
        1 => cache.cache1.dealloc(guard, addr),
        2 => cache.cache2.dealloc(guard, addr),
        3 => cache.cache3.dealloc(guard, addr),
        4 => cache.cache4.dealloc(guard, addr),
        _ => super::pools::dealloc(guard, [(addr, size)].into_iter()),
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{irq, mm, prelude::ktest};

    use super::{CacheArray, CacheOfSizes};
    use crate::{
        chunk,
        pools::test::TestGlobalPool,
        test::{MockMemoryRegion, TestBuddySet},
    };

    #[ktest]
    fn fragmented_refill_returns_requested_segment() {
        let mut caches = CacheOfSizes::new();
        assert_fragmented_refill(&mut caches.cache1);
        assert_fragmented_refill(&mut caches.cache2);
        assert_fragmented_refill(&mut caches.cache3);
        assert_fragmented_refill(&mut caches.cache4);
    }

    #[track_caller]
    fn assert_fragmented_refill<const SEGMENT_PAGES: usize, const COUNT: usize>(
        cache: &mut CacheArray<SEGMENT_PAGES, COUNT>,
    ) {
        const REGION_PAGES: usize = 32;
        const STRIDE: usize = 8;
        const FREE_PAGES: usize = REGION_PAGES - REGION_PAGES / STRIDE;

        let region = MockMemoryRegion::alloc(REGION_PAGES * mm::PAGE_SIZE);
        let base = region.paddr();
        let _irq = irq::disable_local();
        let mut local = TestBuddySet::new_empty();
        let mut global = TestGlobalPool::new();

        // Regression for #3956: withholding one page in eight leaves 28 free
        // pages in total, but no contiguous range large enough for any batch.
        for group in (0..REGION_PAGES).step_by(STRIDE) {
            let start = base + (group + 1) * mm::PAGE_SIZE;
            for (addr, order) in chunk::split_to_chunks(start, (STRIDE - 1) * mm::PAGE_SIZE) {
                local.insert_chunk(addr, order);
            }
        }
        assert_eq!(local.total_size(), FREE_PAGES * mm::PAGE_SIZE);

        let mut attempts = 0;
        let addr = cache
            .alloc_with(|layout| {
                attempts += 1;
                assert_eq!(layout.align(), mm::PAGE_SIZE);
                let result = global.attempt(&mut local, layout);
                if attempts == 1 {
                    assert!(layout.size() > SEGMENT_PAGES * mm::PAGE_SIZE);
                    assert_eq!(result, None);
                } else {
                    assert_eq!(layout.size(), SEGMENT_PAGES * mm::PAGE_SIZE);
                }
                result
            })
            .expect("one segment fits between the retained pages");
        assert_eq!(attempts, 2);
        assert_eq!(cache.size, 0);
        assert!(addr.is_multiple_of(mm::PAGE_SIZE));
        assert_eq!(
            local.total_size() + global.total_size(),
            (FREE_PAGES - SEGMENT_PAGES) * mm::PAGE_SIZE
        );

        // Draining both pools also checks that a three-page request returned
        // the unused tail of its four-page buddy chunk.
        let mut seen = [false; REGION_PAGES];
        let first = (addr - base) / mm::PAGE_SIZE;
        seen[first..first + SEGMENT_PAGES].fill(true);
        while let Some(addr) = local.alloc_chunk(0).or_else(|| global.alloc_chunk(0)) {
            let page = (addr - base) / mm::PAGE_SIZE;
            assert!(!seen[page]);
            seen[page] = true;
        }
        for (page, used) in seen.into_iter().enumerate() {
            assert_eq!(used, !page.is_multiple_of(STRIDE));
        }
        assert_eq!(
            cache.alloc_with(|layout| global.attempt(&mut local, layout)),
            None
        );
    }
}
