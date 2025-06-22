// SPDX-License-Identifier: MPL-2.0

//! Kernel virtual memory allocation

use core::ops::Range;

use super::{KERNEL_PAGE_TABLE, VMALLOC_VADDR_RANGE};
use crate::{
    mm::{
        frame::{meta::AnyFrameMeta, Frame},
        kspace::{KernelPtConfig, MappedItem},
        page_prop::PageProperty,
        page_table::largest_pages,
        Paddr, Vaddr, PAGE_SIZE,
    },
    task::disable_preempt,
    util::range_alloc::RangeAllocator,
};

static KVIRT_AREA_ALLOCATOR: RangeAllocator = RangeAllocator::new(VMALLOC_VADDR_RANGE);

/// Kernel virtual area.
///
/// A kernel virtual area manages a range of memory in [`VMALLOC_VADDR_RANGE`].
/// It can map a portion or the entirety of its virtual memory pages to
/// physical memory, whether tracked with metadata or not.
///
/// It is the caller's responsibility to ensure TLB coherence before using the
/// mapped virtual address on a certain CPU.
//
// FIXME: This caller-ensured design is very error-prone. A good option is to
// use a guard the pins the CPU and ensures TLB coherence while accessing the
// `KVirtArea`. However, `IoMem` need some non trivial refactoring to support
// being implemented on a `!Send` and `!Sync` guard.
#[derive(Debug)]
pub struct KVirtArea {
    range: Range<Vaddr>,
}

impl KVirtArea {
    pub fn start(&self) -> Vaddr {
        self.range.start
    }

    pub fn end(&self) -> Vaddr {
        self.range.end
    }

    pub fn range(&self) -> Range<Vaddr> {
        self.range.start..self.range.end
    }

    #[cfg(ktest)]
    pub fn len(&self) -> usize {
        self.range.len()
    }

    #[cfg(ktest)]
    pub fn query(&self, addr: Vaddr) -> Option<super::MappedItem> {
        use align_ext::AlignExt;

        assert!(self.start() <= addr && self.end() >= addr);
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let preempt_guard = disable_preempt();
        let mut cursor = page_table.cursor(&preempt_guard, &vaddr).unwrap();
        cursor.query().unwrap().1
    }

    /// Create a kernel virtual area and map tracked pages into it.
    ///
    /// The created virtual area will have a size of `area_size`, and the pages
    /// will be mapped starting from `map_offset` in the area.
    ///
    /// # Panics
    ///
    /// This function panics if
    ///  - the area size is not a multiple of [`PAGE_SIZE`];
    ///  - the map offset is not aligned to [`PAGE_SIZE`];
    ///  - the map offset plus the size of the pages exceeds the area size.
    pub fn map_frames<T: AnyFrameMeta>(
        area_size: usize,
        map_offset: usize,
        frames: impl Iterator<Item = Frame<T>>,
        prop: PageProperty,
    ) -> Self {
        assert!(area_size % PAGE_SIZE == 0);
        assert!(map_offset % PAGE_SIZE == 0);

        let range = KVIRT_AREA_ALLOCATOR.alloc(area_size).unwrap();
        let cursor_range = range.start + map_offset..range.end;

        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let preempt_guard = disable_preempt();
        let mut cursor = page_table
            .cursor_mut(&preempt_guard, &cursor_range)
            .unwrap();

        for frame in frames.into_iter() {
            // SAFETY: The constructor of the `KVirtArea` has already ensured
            // that this mapping does not affect kernel's memory safety.
            unsafe { cursor.map(MappedItem::Tracked(frame.into(), prop)) }
                .expect("Failed to map frame in a new `KVirtArea`");
        }

        Self { range }
    }

    /// Creates a kernel virtual area and maps untracked frames into it.
    ///
    /// The created virtual area will have a size of `area_size`, and the
    /// physical addresses will be mapped starting from `map_offset` in
    /// the area.
    ///
    /// You can provide a `0..0` physical range to create a virtual area without
    /// mapping any physical memory.
    ///
    /// # Panics
    ///
    /// This function panics if
    ///  - the area size is not a multiple of [`PAGE_SIZE`];
    ///  - the map offset is not aligned to [`PAGE_SIZE`];
    ///  - the provided physical range is not aligned to [`PAGE_SIZE`];
    ///  - the map offset plus the length of the physical range exceeds the
    ///    area size;
    ///  - the provided physical range contains tracked physical addresses.
    pub unsafe fn map_untracked_frames(
        area_size: usize,
        map_offset: usize,
        pa_range: Range<Paddr>,
        prop: PageProperty,
    ) -> Self {
        assert!(pa_range.start % PAGE_SIZE == 0);
        assert!(pa_range.end % PAGE_SIZE == 0);
        assert!(area_size % PAGE_SIZE == 0);
        assert!(map_offset % PAGE_SIZE == 0);
        assert!(map_offset + pa_range.len() <= area_size);

        let range = KVIRT_AREA_ALLOCATOR.alloc(area_size).unwrap();

        if !pa_range.is_empty() {
            let len = pa_range.len();
            let va_range = range.start + map_offset..range.start + map_offset + len;

            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let preempt_guard = disable_preempt();
            let mut cursor = page_table.cursor_mut(&preempt_guard, &va_range).unwrap();

            for (pa, level) in largest_pages::<KernelPtConfig>(va_range.start, pa_range.start, len)
            {
                // SAFETY: The caller of `map_untracked_frames` has ensured the safety of this mapping.
                let _ = unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) };
            }
        }

        Self { range }
    }
}

impl Drop for KVirtArea {
    fn drop(&mut self) {
        // 1. unmap all mapped pages.
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let range = self.start()..self.end();
        let preempt_guard = disable_preempt();
        let mut cursor = page_table.cursor_mut(&preempt_guard, &range).unwrap();
        loop {
            // SAFETY: The range is under `KVirtArea` so it is safe to unmap.
            let Some(frag) = (unsafe { cursor.take_next(self.end() - cursor.virt_addr()) }) else {
                break;
            };
            drop(frag);
        }
        // 2. free the virtual block
        KVIRT_AREA_ALLOCATOR.free(range);
    }
}
