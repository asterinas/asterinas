// aster-frame/mm/kspace/kva.rs
use core::ops::{ Range, DerefMut};
use crate::mm::{Vaddr, page_prop::PageProperty, page::{Page, meta::{PageMeta, PageUsage}}};
use alloc::{vec::Vec, collections::BTreeMap};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    sync::SpinLock,
};
pub struct KvaFreeNode {
    block: Range<Vaddr>,
}

impl KvaFreeNode {
    pub(crate) const fn new(range: Range<Vaddr>) -> Self {
        Self {
            block: range,
        }
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
    fn alloc(&mut self, freelist: &mut BTreeMap<Vaddr, KvaFreeNode>, size: usize) -> Result<Self, KvaAllocError> {
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
            Ok(Self { var: range})
        } else {
            Err(KvaAllocError::OutOfMemory)
        }
    }
    // /// Un-map pages mapped in kernel virtual area.
    // /// # Safety
    // /// The caller should ensure that the operation doesn't violate the memory safety of
    // /// kernel objects.
    // unsafe fn unmap(&mut self, range: Range<Vaddr>, pages: Vec<Page>) {
    //     // 
    //     todo!();
    // }
    /// Set the property of the mapping.
    /// # Safety
    /// The caller should ensure that the protection doesn't violate the memory safety of
    /// kernel objects.
    unsafe fn protect(&mut self, range: Range<Vaddr>, mut op: impl FnMut(&mut PageProperty)) {
        todo!();
    }
    // // `VmIo` counterparts
    // fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
    //     todo!() // implementation provided by this trait.
    // }
    // unsafe fn write_bytes(&mut self, offset: usize, buf: &[u8]) -> Result<()> {
    //     todo!() // implementation provided by this trait.
    // }
    // Maybe other advanced object R/W methods like what's offered in the safe version?
}

// static KVA_FREELIST: SpinLock<BtreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BtreeMap::new(KvaFreeNode::new(kspace::TRACKED_MAPPED_PAGES_RANGE)));

impl Drop for KvaInner {
    fn drop(&mut self) {
        // 0. unmap all mapped pages.
        // 1. get the previous free block, check if we can merge this block with the free one
        //     - if contiguous, merge this area with the free block.
        //     - if not contiguous, create a new free block, insert it into the list.
        // 2. check if we can merge the current block with the next block, if we can, do so.
        todo!();
        let freelist = KVA_FREELIST.lock().deref_mut();
        freelist.insert(self.var.start, KvaFreeNode::new(self.var.start..self.var.end));
    }
}

pub static KVA_FREELIST: SpinLock<BTreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BTreeMap::new());
pub struct Kva(KvaInner);

impl Kva {
    // static KVA_FREELIST: SpinLock<BtreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BtreeMap::new(KvaFreeNode::new(kspace::KVMALLOC_START_VADDR, kspace::KVMALLOC_RANGE)));
    

    pub fn alloc(&mut self, size: usize) {
        let mut lock_guard = KVA_FREELIST.lock();
        self.0.alloc(lock_guard.deref_mut(), size);
    }
    /// Map pages into the kernel virtual area.
    /// # Safety
    /// The caller should ensure either the mapped pages or the range to be used doesn't
    /// violate the memory safety of kernel objects.
    pub unsafe fn map_pages<T: PageMeta>(&mut self, range: Range<Vaddr>, pages: Vec<Page<T>>) {
        // let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        todo!();
    }
    /// Get the type of the mapped page.
    pub fn get_page_type(&self, addr: Vaddr) -> PageUsage {
        todo!();
    }
    // /// Get the mapped page.
    // /// The method will fail if the provided page type doesn't match the actual mapped one.
    // pub fn get_page<T: PageMeta>(&self, addr: Vaddr) -> Result<Page<T>> {
    //     todo!();
    // }
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