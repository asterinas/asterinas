// SPDX-License-Identifier: MPL-2.0

//! Kernel virtual memory allocation

use alloc::{collections::BTreeMap, vec::Vec};
use core::ops::{DerefMut, Range};

use align_ext::AlignExt;

use super::{KERNEL_PAGE_TABLE, TRACKED_MAPPED_PAGES_RANGE};
use crate::{
    arch::mm::tlb_flush_addr_range,
    mm::{
        page::{
            meta::{PageMeta, PageUsage},
            Page,
        },
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
        page_table::PageTableItem,
        Vaddr, PAGE_SIZE,
    },
    sync::SpinLock,
    Error, Result,
};
pub(crate) use lazy_static::lazy_static;

pub struct KvaFreeNode {
    block: Range<Vaddr>,
}

impl KvaFreeNode {
    pub(crate) const fn new(range: Range<Vaddr>) -> Self {
        Self { block: range }
    }
}

pub struct VirtAddrAllocator {
    freelist: BTreeMap<Vaddr, KvaFreeNode>,
}

impl VirtAddrAllocator {
    fn new(range: Range<Vaddr>) -> Self {
        let mut freelist:BTreeMap<Vaddr, KvaFreeNode> = BTreeMap::new();
        freelist.insert(range.start, KvaFreeNode::new(range));
        Self { freelist }
    }
    /// Allocate a kernel virtual area.
    ///
    /// This is currently implemented with a simple FIRST-FIT algorithm.
    fn alloc(&mut self, size: usize) -> Result<Range<Vaddr>> {
        let mut allocate_range = None;
        let mut to_remove = None;

        for (key, value) in self.freelist.iter() {
            if value.block.end - value.block.start >= size {
                allocate_range = Some((value.block.end - size)..value.block.end);
                to_remove = Some(*key);
                break;
            }
        }

        if let Some(key) = to_remove {
            if let Some(freenode) = self.freelist.get_mut(&key) {
                if freenode.block.end - size == freenode.block.start {
                    self.freelist.remove(&key);
                } else {
                    freenode.block.end -= size;
                }
            }
        }

        if let Some(range) = allocate_range {
            Ok(range)
        } else {
            Err(Error::KvaAllocError)
        }
    }

    /// Free a kernel virtual area.
    fn free(&mut self, range: Range<Vaddr>) {
        // 1. get the previous free block, check if we can merge this block with the free one
        //     - if contiguous, merge this area with the free block.
        //     - if not contiguous, create a new free block, insert it into the list.
        // 2. check if we can merge the current block with the next block, if we can, do so.
        self.freelist.insert(range.start, KvaFreeNode::new(range));
        todo!();
    }
}

lazy_static! {
    pub static ref KVA_ALLOCATOR: SpinLock<VirtAddrAllocator> = SpinLock::new(VirtAddrAllocator::new(TRACKED_MAPPED_PAGES_RANGE));
}

#[derive(Debug)]
pub struct Kva(Range<Vaddr>);

impl Kva {
    // static KVA_FREELIST_2: SpinLock<BTreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BTreeMap::new());

    pub fn new(size: usize) -> Self {
        let mut lock_guard = KVA_ALLOCATOR.lock();
        let var = lock_guard.deref_mut().alloc(size).unwrap();
        Kva(var)
    }

    pub fn start(&self) -> Vaddr {
        self.0.start
    }

    pub fn end(&self) -> Vaddr {
        self.0.end
    }

    pub fn range(&self) -> Range<Vaddr> {
        self.0.start..self.0.end
    }

    /// Map pages into the kernel virtual area.
    /// # Safety
    /// The caller should ensure either the mapped pages or the range to be used doesn't
    /// violate the memory safety of kernel objects.
    pub unsafe fn map_pages<T: PageMeta>(&mut self, range: Range<Vaddr>, pages: Vec<Page<T>>) {
        assert!(range.len() == pages.len() * PAGE_SIZE);
        assert!(self.start() <= range.start && self.end() >= range.end);
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        for page in pages.into_iter() {
            cursor.map(page.into(), prop);
        }
        tlb_flush_addr_range(&range);
    }

    /// This function returns the page usage type based on the provided virtual address `addr`.
    /// This function will fail in the following cases:
    /// * If the address is not mapped (`NotMapped`), the function will fail.
    /// * If the address is mapped to a `MappedUntracked` page, the function will fail.
    pub fn get_page_type(&self, addr: Vaddr) -> PageUsage {
        assert!(self.start() <= addr && self.end() >= addr);
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor(&vaddr).unwrap();
        let query_result = cursor.query().unwrap();
        match query_result {
            PageTableItem::Mapped {
                va: _,
                page,
                prop: _,
            } => page.usage(),
            _ => {
                panic!(
                    "Unexpected query result: Expected 'Mapped', found '{:?}'",
                    query_result
                );
            }
        }
    }

    /// Get the mapped page.
    /// This function will fail in the following cases:
    /// * if the provided page type doesn't match the actual mapped one.
    /// * If the address is not mapped (`NotMapped`), the function will fail.
    /// * If the address is mapped to a `MappedUntracked` page, the function will fail.
    pub fn get_page<T: PageMeta>(&self, addr: Vaddr) -> Result<Page<T>> {
        assert!(self.start() <= addr && self.end() >= addr);
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor(&vaddr).unwrap();
        let query_result = cursor.query().unwrap();
        match query_result {
            PageTableItem::Mapped {
                va: _,
                page,
                prop: _,
            } => {
                let result = Page::<T>::try_from(page);
                if let Ok(page) = result {
                    Ok(page)
                } else {
                    panic!("the provided page type doesn't match the actual mapped one");
                }
            }
            _ => {
                panic!(
                    "Unexpected query result: Expected 'Mapped', found '{:?}'",
                    query_result
                );
            }
        }
    }
}

impl Drop for Kva {
    fn drop(&mut self) {
        // 1. unmap all mapped pages.
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let range = self.start()..self.end();
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        unsafe {
            cursor.unmap(range.len());
        }
        tlb_flush_addr_range(&range);
        // 2. free the virtual block
        let mut lock_guard = KVA_ALLOCATOR.lock();
        lock_guard.deref_mut().free(range);
    }
}