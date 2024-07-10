// aster-frame/mm/kspace/kva.rs
use core::ops::{ Range, DerefMut};
use crate::mm::{page::{self, cont_pages::ContPages, meta::{PageMeta, PageUsage}, Page}, page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags}, Vaddr, VmIo, PAGE_SIZE};
use alloc::{collections::BTreeMap, vec::{self, Vec}};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    sync::SpinLock,
};
use crate::prelude::println;
use super::KERNEL_PAGE_TABLE;

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
    fn alloc(freelist: &mut BTreeMap<Vaddr, KvaFreeNode>, size: usize) -> Result<Self, KvaAllocError> {
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

    fn dealloc(&self, freelist: &mut BTreeMap<Vaddr, KvaFreeNode>, range:Range<Vaddr>)  {
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
pub static KVA_FREELIST: SpinLock<BTreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BTreeMap::new());

// impl Drop for KvaInner {
//     fn drop(&mut self) {
//         // // 0. unmap all mapped pages.
//         // 1. get the previous free block, check if we can merge this block with the free one
//         //     - if contiguous, merge this area with the free block.
//         //     - if not contiguous, create a new free block, insert it into the list.
//         // 2. check if we can merge the current block with the next block, if we can, do so.
//         // todo!();  
//         let mut lock_guard = KVA_FREELIST.lock();
//         lock_guard.deref_mut().insert(self.var.start, KvaFreeNode::new(self.var.start..self.var.end));
//     }
// }

pub struct Kva(KvaInner);

impl Kva {
    // static KVA_FREELIST: SpinLock<BtreeMap<Vaddr, KvaFreeNode>> = SpinLock::new(BtreeMap::new(KvaFreeNode::new(kspace::KVMALLOC_START_VADDR, kspace::KVMALLOC_RANGE)));

    pub fn alloc(size: usize) -> Self {
        let mut lock_guard = KVA_FREELIST.lock();
        let inner = KvaInner::alloc(lock_guard.deref_mut(), size).unwrap();
        Kva(inner)
    }

    pub fn start(& self) -> Vaddr {
        self.0.var.start
    }

    pub fn end(& self) -> Vaddr {
        self.0.var.end
    }

    /// Map pages into the kernel virtual area.
    /// # Safety
    /// The caller should ensure either the mapped pages or the range to be used doesn't
    /// violate the memory safety of kernel objects.
    pub unsafe fn map_pages<T: PageMeta>(&mut self, range: Range<Vaddr>, pages: Vec<Page<T>>) {
        assert!( 
            (range.end - range.start) == (pages.len()*PAGE_SIZE), 
            "The allocated number of frames does not match the required number"
        );
        println!("range.end is : {:X} and range.start is: {:X}", range.end, range.start);
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut va = range.start;
        // page_table
        //         .map(&range, &(pages.start_paddr()..pages.start_paddr()+pages.len()), prop);
        for page in &pages {
            page_table
                .map(
                    &(va..va + PAGE_SIZE), 
                    &(page.paddr()..page.paddr() + PAGE_SIZE), 
                    prop
                ).unwrap();
            va += PAGE_SIZE;
            println!("Page data: {:X}", va);
        }
    }
    /// Get the type of the mapped page.
    pub unsafe fn unmap_pages(&mut self, range: Range<Vaddr>) {
        assert!(
            range.start < self.start() || range.end > self.end(),
            "Unmapping from an invalid address range: start={}, end={}",
            range.start,
            range.end
        );
        let page_table = KERNEL_PAGE_TABLE.get().unwrap();
        unsafe {
            let _ = page_table.unmap(&range); 
        }
    }

    pub fn get_page_type(&self, addr: Vaddr) -> PageUsage {
        todo!();
    }
    // /// Get the mapped page.
    // /// The method will fail if the provided page type doesn't match the actual mapped one.
    // pub fn get_page<T: PageMeta>(&self, addr: Vaddr) -> Result<Page<T>> {
    //     todo!();
    // }
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
        self.0.dealloc(lock_guard.deref_mut(), self.start()..self.end());
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