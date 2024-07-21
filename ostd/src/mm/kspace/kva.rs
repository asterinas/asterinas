// SPDX-License-Identifier: MPL-2.0

//! Kernel virtual memory allocation
use alloc::{collections::BTreeMap, vec::Vec};
use core::ops::{DerefMut, Range};

use align_ext::AlignExt;

use super::KERNEL_PAGE_TABLE;
use crate::{
    arch::mm::tlb_flush_addr_range,
    mm::{
        page::{
            meta::{PageMeta, PageUsage},
            Page,
        },
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
        page_table::PageTableQueryResult,
        Vaddr, VmReader, VmWriter, PAGE_SIZE,
    },
    sync::SpinLock,
    Error, Result,
};

pub struct KvaFreeNode {
    block: Range<Vaddr>,
}

impl KvaFreeNode {
    pub(crate) const fn new(range: Range<Vaddr>) -> Self {
        Self { block: range }
    }
}

pub trait KvaAlloc: Sized {
    /// Create a new kernel virtual area with the given allocated range.
    fn init(vaddr: Range<Vaddr>) -> Self;

    /// Get the range of the kernel virtual area.
    fn range(&self) -> Range<Vaddr>;

    fn start(&self) -> Vaddr {
        self.range().start
    }

    fn end(&self) -> Vaddr {
        self.range().end
    }
    /// Allocate a kernel virtual area.
    ///
    /// This is currently implemented with a simple FIRST-FIT algorithm.
    fn alloc(freelist: &mut BTreeMap<Vaddr, KvaFreeNode>, size: usize) -> Result<Self> {
        let mut allocate_range = None;
        let mut to_remove = None;

        for (key, value) in freelist.iter() {
            if value.block.end - value.block.start >= size {
                allocate_range = Some((value.block.end - size)..value.block.end);
                // if value.block.end - value.block.start == size {
                to_remove = Some(*key);
                // ß}
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
            Ok(Self::init(range))
        } else {
            Err(Error::KvaAllocError)
        }
    }

    /// Free a kernel virtual area.
    fn free(&mut self, freelist: &mut BTreeMap<Vaddr, KvaFreeNode>) {
        freelist.insert(self.start(), KvaFreeNode::new(self.range()));
    }

    unsafe fn protect(&mut self, range: Range<Vaddr>, op: impl FnMut(&mut PageProperty)) {
        assert!(range.start >= self.start() && range.end <= self.end());
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        cursor.protect(range.len(), op, true).unwrap();
    }

    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.range().len() {
            return Err(Error::InvalidArgs);
        }
        let reader =
            unsafe { VmReader::from_kernel_space(self.start() as *const u8, self.range().len()) };
        let len = reader.skip(offset).read(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.range().len() {
            return Err(Error::InvalidArgs);
        }
        let writer =
            unsafe { VmWriter::from_kernel_space(self.start() as *mut u8, self.range().len()) };
        let len = writer.skip(offset).write(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }
}

pub static KVA_FREELIST: SpinLock<BTreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BTreeMap::new());

pub struct Kva {
    var: Range<Vaddr>,
}

impl KvaAlloc for Kva {
    /// Create a new kernel virtual area with the given allocated range.
    fn init(vaddr: Range<Vaddr>) -> Self {
        Self { var: vaddr }
    }

    /// Get the range of the kernel virtual area.
    fn range(&self) -> Range<Vaddr> {
        self.var.start..self.var.end
    }
}

impl Kva {
    pub fn new(size: usize) -> Self {
        let mut lock_guard = KVA_FREELIST.lock();
        Self::alloc(lock_guard.deref_mut(), size).unwrap()
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

    pub fn get_page_type(&self, addr: Vaddr) -> PageUsage {
        assert!(self.start() <= addr && self.end() >= addr);
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor(&vaddr).unwrap();
        let query_result = cursor.query().unwrap();
        match query_result {
            PageTableQueryResult::Mapped {
                va: _,
                page,
                prop: _,
            } => page.usage(),
            _ => {
                //  MappedUntracked and NotMapped
                panic!(
                    "Unexpected query result: Expected 'Mapped', found '{:?}'",
                    query_result
                );
            }
        }
    }
    /// Get the mapped page.
    /// The method will fail if the provided page type doesn't match the actual mapped one.
    pub fn get_page<T: PageMeta>(&self, addr: Vaddr) -> Result<Page<T>> {
        assert!(self.start() <= addr && self.end() >= addr);
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor(&vaddr).unwrap();
        let query_result = cursor.query().unwrap();
        match query_result {
            PageTableQueryResult::Mapped {
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
                //  MappedUntracked and NotMapped
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
        // 0. unmap all mapped pages.
        // 1. get the previous free block, check if we can merge this block with the free one
        //     - if contiguous, merge this area with the free block.
        //     - if not contiguous, create a new free block, insert it into the list.
        // 2. check if we can merge the current block with the next block, if we can, do so.
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let range = self.range();
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        unsafe {
            cursor.unmap(range.len());
        }
        tlb_flush_addr_range(&range);
        let mut lock_guard = KVA_FREELIST.lock();
        self.free(lock_guard.deref_mut());
    }
}
