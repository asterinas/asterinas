// SPDX-License-Identifier: MPL-2.0

//! Kernel virtual memory allocation
use align_ext::AlignExt;
use alloc::{collections::BTreeMap, vec::Vec};
use core::ops::{DerefMut, Range};

use super::KERNEL_PAGE_TABLE;
use crate::{
    arch::mm::tlb_flush_addr_range,
    mm::{
        page::{
            meta::{PageMeta, PageUsage},
            Page,
        },
        page_table::PageTableQueryResult,
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
        Vaddr, VmIo, PAGE_SIZE, VmReader, VmWriter,
    },
    sync::SpinLock, Error, Result,
};

pub struct KvaFreeNode {
    block: Range<Vaddr>,
}

impl KvaFreeNode {
    pub(crate) const fn new(range: Range<Vaddr>) -> Self {
        Self { block: range }
    }
}

#[derive(Debug)]
pub enum KvaAllocError {
    OutOfMemory,
}

struct KvaInner {
    /// The virtual memory range.
    var: Range<Vaddr>,
}

// No reference counting. And no `Clone` implementation here.
// The user may wrap it in a `Arc` to allow shared accesses.

// aster-frame/mm/kspace/kva.rs
impl KvaInner {
    /// Allocate a kernel virtual area.
    /// This is a simple FIRST-FIT algorithm.
    /// Note that a newly allocated area is not backed by any physical pages.
    fn alloc(
        freelist: &mut BTreeMap<Vaddr, KvaFreeNode>,
        size: usize,
    ) -> Result<Self> {
        // iterate through the free list, if find the first block that is larger than this allocation, do:
        //    1. consume the last part of this block as the allocated range.
        //    2. check if this block is empty (and not the first block), if so, remove it.
        // if exhausted, return error.
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
            Ok(Self { var: range })
        } else {
            Err(Error::KvaAllocError)
        }
    }

    fn dealloc(&self, freelist: &mut BTreeMap<Vaddr, KvaFreeNode>, range: Range<Vaddr>) {
        freelist.insert(range.start, KvaFreeNode::new(range.start..range.end));
    }
    // /// Un-map pages mapped in kernel virtual area.
    // /// # Safety
    // /// The caller should ensure that the operation doesn't violate the memory safety of
    // /// kernel objects.
    // unsafe fn unmap<T>(&mut self, range: Range<Vaddr>, pages: Vec<Page<T>>) {
    //     //
    //     todo!();
    // }
    /// Set the property of the mapping.
    /// # Safety
    /// The caller should ensure that the protection doesn't violate the memory safety of
    /// kernel objects.
    unsafe fn protect(&mut self, range: Range<Vaddr>, op: impl FnMut(&mut PageProperty)) {
        assert!(range.start >= self.var.start && range.end <= self.var.end);
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        cursor.protect(range.len(), op, true).unwrap();
    }

    // Maybe other advanced object R/W methods like what's offered in the safe version?
}

impl<'a> KvaInner {
    /// Returns a reader to read data from it.
    pub fn reader(&'a self) -> VmReader<'a> {
        unsafe { VmReader::from_kernel_space(self.var.start as *const u8, self.var.len()) }
    }

    /// Returns a writer to write data into it.
    pub fn writer(&'a self) -> VmWriter<'a> {
        unsafe { VmWriter::from_kernel_space(self.var.start as *mut u8, self.var.len()) }
    }
}

impl VmIo for KvaInner {
    // `VmIo` counterparts
    fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.var.len() {
            return Err(Error::InvalidArgs);
        }
        let len = self.reader().skip(offset).read(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }

    fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        // Do bound check with potential integer overflow in mind
        let max_offset = offset.checked_add(buf.len()).ok_or(Error::Overflow)?;
        if max_offset > self.var.len() {
            return Err(Error::InvalidArgs);
        }
        let len = self.writer().skip(offset).write(&mut buf.into());
        debug_assert!(len == buf.len());
        Ok(())
    }
}

// static KVA_FREELIST: SpinLock<BtreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BtreeMap::new(KvaFreeNode::new(kspace::TRACKED_MAPPED_PAGES_RANGE)));
pub static KVA_FREELIST: SpinLock<BTreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BTreeMap::new());

pub struct Kva(KvaInner);

impl Kva {
    // static KVA_FREELIST: SpinLock<BtreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BtreeMap::new(KvaFreeNode::new(kspace::KVMALLOC_START_VADDR, kspace::KVMALLOC_RANGE)));

    pub fn alloc(size: usize) -> Self {
        let mut lock_guard = KVA_FREELIST.lock();
        let inner = KvaInner::alloc(lock_guard.deref_mut(), size).unwrap();
        Kva(inner)
    }

    pub fn start(&self) -> Vaddr {
        self.0.var.start
    }

    pub fn end(&self) -> Vaddr {
        self.0.var.end
    }

    /// Map pages into the kernel virtual area.
    /// # Safety
    /// The caller should ensure either the mapped pages or the range to be used doesn't
    /// violate the memory safety of kernel objects.
    pub unsafe fn map_pages<T: PageMeta>(&mut self, range: Range<Vaddr>, pages: Vec<Page<T>>) {
        assert!(range.len() == pages.len() * PAGE_SIZE);
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        // page_table
        //         .map(&range, &(pages.start_paddr()..pages.start_paddr()+pages.len()), prop);
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        for page in pages.into_iter() {
            cursor.map(page.into(), prop);
        }
        tlb_flush_addr_range(&range);
    }
    /// Get the type of the mapped page.
    pub unsafe fn unmap_pages(&mut self, range: Range<Vaddr>) {
        assert!(range.start >= self.start() && range.end <= self.end());
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor_mut(&range).unwrap();
        unsafe {
            cursor.unmap(range.len());
        }
        tlb_flush_addr_range(&range);
    }

    pub fn get_page_type(&self, addr: Vaddr) -> PageUsage {
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor(&vaddr).unwrap();
        let query_result = cursor.query().unwrap();
        match query_result {
            PageTableQueryResult::Mapped { va : _, page, prop : _  } => {
               page.usage()
            }
            _ => {
               //  MappedUntracked and NotMapped
               panic!("Unexpected query result: Expected 'Mapped', found '{:?}'", query_result);
            }
        }
    }
    /// Get the mapped page.
    /// The method will fail if the provided page type doesn't match the actual mapped one.
    pub fn get_page<T: PageMeta>(&self, addr: Vaddr) -> Result<Page<T>> {
        let start = addr.align_down(PAGE_SIZE);
        let vaddr = start..start + PAGE_SIZE;
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let mut cursor = page_table.cursor(&vaddr).unwrap();
        // cannot obtain a page through querying the page table at next time?
        let query_result = cursor.query().unwrap();
        match query_result {
            PageTableQueryResult::Mapped { va : _, page, prop : _  } => {
                let result = Page::<T>::try_from(page);
                if let Ok(page) = result {
                    Ok(page)
                } else {
                    panic!("the provided page type doesn't match the actual mapped one");
                }
            }
            _ => {
               //  MappedUntracked and NotMapped
               panic!("Unexpected query result: Expected 'Mapped', found '{:?}'", query_result);
            }
        }
    }
}

impl Drop for Kva {
    fn drop(&mut self) {
        // // 0. unmap all mapped pages.
        // 1. get the previous free block, check if we can merge this block with the free one
        //     - if contiguous, merge this area with the free block.
        //     - if not contiguous, create a new free block, insert it into the list.
        // 2. check if we can merge the current block with the next block, if we can, do so.
        // todo!();
        let mut lock_guard = KVA_FREELIST.lock();
        self.0
            .dealloc(lock_guard.deref_mut(), self.start()..self.end());
        // lock_guard.deref_mut().insert(self.var.start, KvaFreeNode::new(self.var.start..self.var.end));
    }
}

// pub struct IoVa(KvaInner);

// impl IoVa {
//     static IOVA_FREELIST: SpinLock<BtreeMap<KvaFreeNode>> = SpinLock::new(BtreeMap::new(KvaFreeNode::new(kspace::TRACKED_MAPPED_PAGES_RANGE)));
//     pub fn alloc(size: usize) {
//         let lock_guard = IOVA_FREELIST.lock();
//         self.0.alloc(lock_guard.deref_mut(), size);
//     }
//     /// Map untracked physical memory into the kernel virtual area.
//     /// # Safety
//     /// The caller should ensure either the mapped pages or the range to be used doesn't
//     /// violate the memory safety of kernel objects.
//     pub unsafe fn map(&mut self, virt_addr: Vaddr, phys_addr: Paddr, length: usize) {
//         todo!();
//     }
// }
