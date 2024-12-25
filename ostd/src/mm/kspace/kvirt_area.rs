// SPDX-License-Identifier: MPL-2.0

//! Kernel virtual memory allocation

use core::{any::TypeId, marker::PhantomData, ops::Range};

use super::{KERNEL_PAGE_TABLE, TRACKED_MAPPED_PAGES_RANGE, VMALLOC_VADDR_RANGE};
use crate::{
    cpu::CpuSet,
    mm::{
        frame::{meta::AnyFrameMeta, Frame},
        page_prop::PageProperty,
        page_table::PageTableItem,
        tlb::{TlbFlushOp, TlbFlusher, FLUSH_ALL_RANGE_THRESHOLD},
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
/// A tracked kernel virtual area (`KVirtArea<Tracked>`) manages a range of memory in
/// `TRACKED_MAPPED_PAGES_RANGE`. It can map a inner part or all of its virtual memory
/// to some physical tracked pages.
///
/// A untracked kernel virtual area (`KVirtArea<Untracked>`) manages a range of memory in
/// `VMALLOC_VADDR_RANGE`. It can map a inner part or all of its virtual memory to
/// some physical untracked pages.
#[derive(Debug)]
pub struct KVirtArea<M: AllocatorSelector + 'static> {
    range: Range<Vaddr>,
    phantom: PhantomData<M>,
}

impl<M: AllocatorSelector + 'static> KVirtArea<M> {
    pub fn new(size: usize) -> Self {
        let allocator = M::select_allocator();
        let range = allocator.alloc(size).unwrap();
        Self {
            range,
            phantom: PhantomData,
        }
    }

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
        let mut cursor = page_table.cursor(&vaddr).unwrap();
        cursor.query().unwrap()
    }
}

impl KVirtArea<Tracked> {
    /// Maps pages into the kernel virtual area.
    pub fn map_pages<T: AnyFrameMeta>(
        &mut self,
        range: Range<Vaddr>,
        pages: impl Iterator<Item = Frame<T>>,
        prop: PageProperty,
    ) {
        assert!(self.start() <= range.start && self.end() >= range.end);
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        let mut flusher = TlbFlusher::new(CpuSet::new_full(), disable_preempt());
        let mut va = self.start();
        for page in pages.into_iter() {
            // SAFETY: The constructor of the `KVirtArea<Tracked>` structure has already ensured this
            // mapping does not affect kernel's memory safety.
            if let Some(old) = unsafe { cursor.map(page.into(), prop) } {
                flusher.issue_tlb_flush_with(TlbFlushOp::Address(va), old);
                flusher.dispatch_tlb_flush();
                // FIXME: We should synchronize the TLB here. But we may
                // disable IRQs here, making deadlocks very likely.
            }
            va += PAGE_SIZE;
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
    /// Maps untracked pages into the kernel virtual area.
    ///
    /// `pa_range.start` and `pa_range.end` should be aligned to PAGE_SIZE.
    ///
    /// # Safety
    ///
    /// The caller should ensure that
    ///  - the range being mapped does not affect kernel's memory safety;
    ///  - the physical address to be mapped is valid and safe to use;
    ///  - it is allowed to map untracked pages in this virtual address range.
    pub unsafe fn map_untracked_pages(
        &mut self,
        va_range: Range<Vaddr>,
        pa_range: Range<Paddr>,
        prop: PageProperty,
    ) {
        assert!(pa_range.start % PAGE_SIZE == 0);
        assert!(pa_range.end % PAGE_SIZE == 0);
        assert!(va_range.len() == pa_range.len());
        assert!(self.start() <= va_range.start && self.end() >= va_range.end);

        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor_mut(&va_range).unwrap();
        let mut flusher = TlbFlusher::new(CpuSet::new_full(), disable_preempt());
        // SAFETY: The caller of `map_untracked_pages` has ensured the safety of this mapping.
        unsafe {
            cursor.map_pa(&pa_range, prop);
        }
        flusher.issue_tlb_flush(TlbFlushOp::Range(va_range.clone()));
        flusher.dispatch_tlb_flush();
        // FIXME: We should synchronize the TLB here. But we may
        // disable IRQs here, making deadlocks very likely.
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
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        let mut flusher = TlbFlusher::new(CpuSet::new_full(), disable_preempt());
        let tlb_prefer_flush_all = self.end() - self.start() > FLUSH_ALL_RANGE_THRESHOLD;

        loop {
            let result = unsafe { cursor.take_next(self.end() - cursor.virt_addr()) };
            match result {
                PageTableItem::Mapped { va, page, .. } => match TypeId::of::<M>() {
                    id if id == TypeId::of::<Tracked>() => {
                        if !flusher.need_remote_flush() && tlb_prefer_flush_all {
                            // Only on single-CPU cases we can drop the page immediately before flushing.
                            drop(page);
                            continue;
                        }
                        flusher.issue_tlb_flush_with(TlbFlushOp::Address(va), page);
                    }
                    id if id == TypeId::of::<Untracked>() => {
                        panic!("Found tracked memory mapped into untracked `KVirtArea`");
                    }
                    _ => panic!("Unexpected `KVirtArea` type"),
                },
                PageTableItem::MappedUntracked { va, .. } => match TypeId::of::<M>() {
                    id if id == TypeId::of::<Untracked>() => {
                        if !flusher.need_remote_flush() && tlb_prefer_flush_all {
                            continue;
                        }
                        flusher.issue_tlb_flush(TlbFlushOp::Address(va));
                    }
                    id if id == TypeId::of::<Tracked>() => {
                        panic!("Found untracked memory mapped into tracked `KVirtArea`");
                    }
                    _ => panic!("Unexpected `KVirtArea` type"),
                },
                PageTableItem::NotMapped { .. } => {
                    break;
                }
            }
        }

        if !flusher.need_remote_flush() && tlb_prefer_flush_all {
            flusher.issue_tlb_flush(TlbFlushOp::All);
        }

        flusher.dispatch_tlb_flush();
        // FIXME: We should synchronize the TLB here. But we may
        // disable IRQs here, making deadlocks very likely.

        // 2. free the virtual block
        let allocator = M::select_allocator();
        allocator.free(range);
    }
}
