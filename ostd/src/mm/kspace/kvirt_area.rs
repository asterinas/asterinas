// SPDX-License-Identifier: MPL-2.0

//! Kernel virtual memory allocation

use core::{marker::PhantomData, ops::Range};

use super::{KERNEL_PAGE_TABLE, TRACKED_MAPPED_PAGES_RANGE, VMALLOC_VADDR_RANGE};
use crate::{
    mm::{
        frame::{meta::AnyFrameMeta, Frame},
        page_prop::PageProperty,
        page_table::PageTableItem,
        Paddr, Vaddr, PAGE_SIZE,
    },
    task::disable_preempt,
    util::range_alloc::RangeAllocator,
};

static KVIRT_AREA_TRACKED_ALLOCATOR: RangeAllocator =
    RangeAllocator::new(TRACKED_MAPPED_PAGES_RANGE);
static KVIRT_AREA_UNTRACKED_ALLOCATOR: RangeAllocator = RangeAllocator::new(VMALLOC_VADDR_RANGE);

#[derive(Debug)]
pub struct Tracked;
#[derive(Debug)]
pub struct Untracked;

pub trait AllocatorSelector {
    fn select_allocator() -> &'static RangeAllocator;
}

impl AllocatorSelector for Tracked {
    fn select_allocator() -> &'static RangeAllocator {
        &KVIRT_AREA_TRACKED_ALLOCATOR
    }
}

impl AllocatorSelector for Untracked {
    fn select_allocator() -> &'static RangeAllocator {
        &KVIRT_AREA_UNTRACKED_ALLOCATOR
    }
}

/// Kernel Virtual Area.
///
/// A tracked kernel virtual area ([`KVirtArea<Tracked>`]) manages a range of
/// memory in [`TRACKED_MAPPED_PAGES_RANGE`]. It can map a portion or the
/// entirety of its virtual memory pages to frames tracked with metadata.
///
/// An untracked kernel virtual area ([`KVirtArea<Untracked>`]) manages a range
/// of memory in [`VMALLOC_VADDR_RANGE`]. It can map a portion or the entirety
/// of virtual memory to physical addresses not tracked with metadata.
///
/// It is the caller's responsibility to ensure TLB coherence before using the
/// mapped virtual address on a certain CPU.
//
// FIXME: This caller-ensured design is very error-prone. A good option is to
// use a guard the pins the CPU and ensures TLB coherence while accessing the
// `KVirtArea`. However, `IoMem` need some non trivial refactoring to support
// being implemented on a `!Send` and `!Sync` guard.
#[derive(Debug)]
pub struct KVirtArea<M: AllocatorSelector + 'static> {
    range: Range<Vaddr>,
    phantom: PhantomData<M>,
}

impl<M: AllocatorSelector + 'static> KVirtArea<M> {
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
    fn query_page(&self, addr: Vaddr) -> PageTableItem {
        use align_ext::AlignExt;

        assert!(self.start() <= addr && self.end() >= addr);
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let preempt_guard = disable_preempt();
        let mut cursor = page_table.cursor(&preempt_guard, &vaddr).unwrap();
        cursor.query().unwrap()
    }
}

impl KVirtArea<Tracked> {
    /// Create a kernel virtual area and map pages into it.
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
    pub fn map_pages<T: AnyFrameMeta>(
        area_size: usize,
        map_offset: usize,
        pages: impl Iterator<Item = Frame<T>>,
        prop: PageProperty,
    ) -> Self {
        assert!(area_size % PAGE_SIZE == 0);
        assert!(map_offset % PAGE_SIZE == 0);
        let range = Tracked::select_allocator().alloc(area_size).unwrap();
        let cursor_range = range.start + map_offset..range.end;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let preempt_guard = disable_preempt();
        let mut cursor = page_table
            .cursor_mut(&preempt_guard, &cursor_range)
            .unwrap();
        for page in pages.into_iter() {
            // SAFETY: The constructor of the `KVirtArea<Tracked>` structure
            // has already ensured that this mapping does not affect kernel's
            // memory safety.
            if let Some(_old) = unsafe { cursor.map(page.into(), prop) } {
                panic!("Pages mapped in a newly allocated `KVirtArea`");
            }
        }
        Self {
            range,
            phantom: PhantomData,
        }
    }

    /// Gets the mapped tracked page.
    ///
    /// This function returns None if the address is not mapped (`NotMapped`),
    /// while panics if the address is mapped to a `MappedUntracked` or `PageTableNode` page.
    #[cfg(ktest)]
    pub fn get_page(&self, addr: Vaddr) -> Option<Frame<dyn AnyFrameMeta>> {
        let query_result = self.query_page(addr);
        match query_result {
            PageTableItem::Mapped {
                va: _,
                page,
                prop: _,
            } => Some(page),
            PageTableItem::NotMapped { .. } => None,
            _ => {
                panic!(
                    "Found '{:?}' mapped into tracked `KVirtArea`, expected `Mapped`",
                    query_result
                );
            }
        }
    }
}

impl KVirtArea<Untracked> {
    /// Creates a kernel virtual area and maps untracked frames into it.
    ///
    /// The created virtual area will have a size of `area_size`, and the
    /// physical addresses will be mapped starting from `map_offset` in
    /// the area.
    ///
    /// # Panics
    ///
    /// This function panics if
    ///  - the area size is not a multiple of [`PAGE_SIZE`];
    ///  - the map offset is not aligned to [`PAGE_SIZE`];
    ///  - the provided physical range is not aligned to [`PAGE_SIZE`];
    ///  - the map offset plus the length of the physical range exceeds the
    ///    area size.
    pub unsafe fn map_untracked_pages(
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
        let range = Untracked::select_allocator().alloc(area_size).unwrap();
        if !pa_range.is_empty() {
            let va_range = range.start + map_offset..range.start + map_offset + pa_range.len();

            let page_table = KERNEL_PAGE_TABLE.get().unwrap();
            let preempt_guard = disable_preempt();
            let mut cursor = page_table.cursor_mut(&preempt_guard, &va_range).unwrap();
            // SAFETY: The caller of `map_untracked_pages` has ensured the safety of this mapping.
            unsafe {
                cursor.map_pa(&pa_range, prop);
            }
        }
        Self {
            range,
            phantom: PhantomData,
        }
    }

    /// Gets the mapped untracked page.
    ///
    /// This function returns None if the address is not mapped (`NotMapped`),
    /// while panics if the address is mapped to a `Mapped` or `PageTableNode` page.
    #[cfg(ktest)]
    pub fn get_untracked_page(&self, addr: Vaddr) -> Option<(Paddr, usize)> {
        let query_result = self.query_page(addr);
        match query_result {
            PageTableItem::MappedUntracked {
                va: _,
                pa,
                len,
                prop: _,
            } => Some((pa, len)),
            PageTableItem::NotMapped { .. } => None,
            _ => {
                panic!(
                    "Found '{:?}' mapped into untracked `KVirtArea`, expected `MappedUntracked`",
                    query_result
                );
            }
        }
    }
}

impl<M: AllocatorSelector + 'static> Drop for KVirtArea<M> {
    fn drop(&mut self) {
        // 1. unmap all mapped pages.
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let range = self.start()..self.end();
        let preempt_guard = disable_preempt();
        let mut cursor = page_table.cursor_mut(&preempt_guard, &range).unwrap();
        loop {
            let result = unsafe { cursor.take_next(self.end() - cursor.virt_addr()) };
            if matches!(&result, PageTableItem::NotMapped { .. }) {
                break;
            }
            // Dropping previously mapped pages is fine since accessing with
            // the virtual addresses in another CPU while we are dropping is
            // not allowed.
            drop(result);
        }
        // 2. free the virtual block
        let allocator = M::select_allocator();
        allocator.free(range);
    }
}
