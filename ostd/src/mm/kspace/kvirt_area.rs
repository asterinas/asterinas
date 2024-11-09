// SPDX-License-Identifier: MPL-2.0

//! Kernel virtual memory allocation

use alloc::collections::BTreeMap;
use core::{any::TypeId, marker::PhantomData, ops::Range};

use align_ext::AlignExt;

use super::{KERNEL_PAGE_TABLE, TRACKED_MAPPED_PAGES_RANGE, VMALLOC_VADDR_RANGE};
use crate::{
    cpu::CpuSet,
    mm::{
        page::{meta::PageMeta, DynPage, Page},
        page_prop::PageProperty,
        page_table::PageTableItem,
        tlb::{TlbFlushOp, TlbFlusher, FLUSH_ALL_RANGE_THRESHOLD},
        Paddr, Vaddr, PAGE_SIZE,
    },
    sync::SpinLock,
    task::disable_preempt,
    Error, Result,
};

pub struct KVirtAreaFreeNode {
    block: Range<Vaddr>,
}

impl KVirtAreaFreeNode {
    pub(crate) const fn new(range: Range<Vaddr>) -> Self {
        Self { block: range }
    }
}

pub struct VirtAddrAllocator {
    fullrange: Range<Vaddr>,
    freelist: SpinLock<Option<BTreeMap<Vaddr, KVirtAreaFreeNode>>>,
}

impl VirtAddrAllocator {
    const fn new(fullrange: Range<Vaddr>) -> Self {
        Self {
            fullrange,
            freelist: SpinLock::new(None),
        }
    }

    /// Allocates a kernel virtual area.
    ///
    /// This is currently implemented with a simple FIRST-FIT algorithm.
    fn alloc(&self, size: usize) -> Result<Range<Vaddr>> {
        let mut lock_guard = self.freelist.lock();
        if lock_guard.is_none() {
            let mut freelist: BTreeMap<Vaddr, KVirtAreaFreeNode> = BTreeMap::new();
            freelist.insert(
                self.fullrange.start,
                KVirtAreaFreeNode::new(self.fullrange.clone()),
            );
            *lock_guard = Some(freelist);
        }
        let freelist = lock_guard.as_mut().unwrap();
        let mut allocate_range = None;
        let mut to_remove = None;

        for (key, value) in freelist.iter() {
            if value.block.end - value.block.start >= size {
                allocate_range = Some((value.block.end - size)..value.block.end);
                to_remove = Some(*key);
                break;
            }
        }

        if let Some(key) = to_remove {
            if let Some(freenode) = freelist.get_mut(&key) {
                if freenode.block.end - size == freenode.block.start {
                    freelist.remove(&key);
                } else {
                    freenode.block.end -= size;
                }
            }
        }

        if let Some(range) = allocate_range {
            Ok(range)
        } else {
            Err(Error::KVirtAreaAllocError)
        }
    }

    /// Frees a kernel virtual area.
    fn free(&self, range: Range<Vaddr>) {
        let mut lock_guard = self.freelist.lock();
        let freelist = lock_guard.as_mut().unwrap_or_else(|| {
            panic!("Free a 'KVirtArea' when 'VirtAddrAllocator' has not been initialized.")
        });
        // 1. get the previous free block, check if we can merge this block with the free one
        //     - if contiguous, merge this area with the free block.
        //     - if not contiguous, create a new free block, insert it into the list.
        let mut free_range = range.clone();

        if let Some((prev_va, prev_node)) = freelist
            .upper_bound_mut(core::ops::Bound::Excluded(&free_range.start))
            .peek_prev()
        {
            if prev_node.block.end == free_range.start {
                let prev_va = *prev_va;
                free_range.start = prev_node.block.start;
                freelist.remove(&prev_va);
            }
        }
        freelist.insert(free_range.start, KVirtAreaFreeNode::new(free_range.clone()));

        // 2. check if we can merge the current block with the next block, if we can, do so.
        if let Some((next_va, next_node)) = freelist
            .lower_bound_mut(core::ops::Bound::Excluded(&free_range.start))
            .peek_next()
        {
            if free_range.end == next_node.block.start {
                let next_va = *next_va;
                free_range.end = next_node.block.end;
                freelist.remove(&next_va);
                freelist.get_mut(&free_range.start).unwrap().block.end = free_range.end;
            }
        }
    }
}

static KVIRT_AREA_TRACKED_ALLOCATOR: VirtAddrAllocator =
    VirtAddrAllocator::new(TRACKED_MAPPED_PAGES_RANGE);
static KVIRT_AREA_UNTRACKED_ALLOCATOR: VirtAddrAllocator =
    VirtAddrAllocator::new(VMALLOC_VADDR_RANGE);

#[derive(Debug)]
pub struct Tracked;
#[derive(Debug)]
pub struct Untracked;

pub trait AllocatorSelector {
    fn select_allocator() -> &'static VirtAddrAllocator;
}

impl AllocatorSelector for Tracked {
    fn select_allocator() -> &'static VirtAddrAllocator {
        &KVIRT_AREA_TRACKED_ALLOCATOR
    }
}

impl AllocatorSelector for Untracked {
    fn select_allocator() -> &'static VirtAddrAllocator {
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

    pub fn len(&self) -> usize {
        self.range.len()
    }

    fn query_page(&self, addr: Vaddr) -> PageTableItem {
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
    pub fn map_pages<T: PageMeta>(
        &mut self,
        range: Range<Vaddr>,
        pages: impl Iterator<Item = Page<T>>,
        prop: PageProperty,
    ) {
        assert!(self.start() <= range.start && self.end() >= range.end);
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        let flusher = TlbFlusher::new(CpuSet::new_full(), disable_preempt());
        let mut va = self.start();
        for page in pages.into_iter() {
            // SAFETY: The constructor of the `KVirtArea<Tracked>` structure has already ensured this
            // mapping does not affect kernel's memory safety.
            if let Some(old) = unsafe { cursor.map(page.into(), prop) } {
                flusher.issue_tlb_flush_with(TlbFlushOp::Address(va), old);
                flusher.dispatch_tlb_flush();
            }
            va += PAGE_SIZE;
        }
        flusher.issue_tlb_flush(TlbFlushOp::Range(range));
        flusher.dispatch_tlb_flush();
    }

    /// Gets the mapped tracked page.
    ///
    /// This function returns None if the address is not mapped (`NotMapped`),
    /// while panics if the address is mapped to a `MappedUntracked` or `PageTableNode` page.
    pub fn get_page(&self, addr: Vaddr) -> Option<DynPage> {
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
        let flusher = TlbFlusher::new(CpuSet::new_full(), disable_preempt());
        // SAFETY: The caller of `map_untracked_pages` has ensured the safety of this mapping.
        unsafe {
            cursor.map_pa(&pa_range, prop);
        }
        flusher.issue_tlb_flush(TlbFlushOp::Range(va_range.clone()));
        flusher.dispatch_tlb_flush();
    }

    /// Gets the mapped untracked page.
    ///
    /// This function returns None if the address is not mapped (`NotMapped`),
    /// while panics if the address is mapped to a `Mapped` or `PageTableNode` page.
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
        let flusher = TlbFlusher::new(CpuSet::new_full(), disable_preempt());
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

        // 2. free the virtual block
        let allocator = M::select_allocator();
        allocator.free(range);
    }
}
