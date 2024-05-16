// SPDX-License-Identifier: MPL-2.0

//! Because that the page table implementation requires metadata initialized
//! and mapped, the boot page table is needed to do early stage page table setup
//! in order to initialize the running phase page tables.

use alloc::vec::Vec;

use super::{pte_index, PageTableEntryTrait};
use crate::vm::{
    frame::allocator::FRAME_ALLOCATOR, paddr_to_vaddr, Paddr, PageProperty, PagingConstsTrait,
    Vaddr, PAGE_SIZE,
};

type FrameNumber = usize;

/// A simple boot page table for boot stage mapping management.
/// If applicable, the boot page table could track the lifetime of page table
/// frames that are set up by the firmware, loader or the setup code.
pub struct BootPageTable<E: PageTableEntryTrait, C: PagingConstsTrait> {
    root_pt: FrameNumber,
    // The frames allocated for this page table are not tracked with
    // metadata [`crate::vm::frame::meta`]. Here is a record of it
    // for deallocation.
    frames: Vec<FrameNumber>,
    _pretend_to_use: core::marker::PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> BootPageTable<E, C> {
    /// Create a new boot page table from the a page table root physical address.
    /// The anonymous page table may be set up by the firmware, loader or the setup code.
    pub fn from_anonymous_boot_pt(root_paddr: Paddr) -> Self {
        Self {
            root_pt: root_paddr / C::BASE_PAGE_SIZE,
            frames: Vec::new(),
            _pretend_to_use: core::marker::PhantomData,
        }
    }

    /// Map a base page to a frame.
    /// This function will panic if the page is already mapped.
    pub fn map_base_page(&mut self, from: Vaddr, to: FrameNumber, prop: PageProperty) {
        let mut pt = self.root_pt;
        let mut level = C::NR_LEVELS;
        // Walk to the last level of the page table.
        while level > 1 {
            let index = pte_index::<C>(from, level);
            let pte_ptr = unsafe { (paddr_to_vaddr(pt * C::BASE_PAGE_SIZE) as *mut E).add(index) };
            let pte = unsafe { pte_ptr.read() };
            pt = if !pte.is_present() {
                let frame = self.alloc_frame();
                unsafe { pte_ptr.write(E::new_pt(frame * C::BASE_PAGE_SIZE)) };
                frame
            } else if pte.is_last(level) {
                panic!("mapping an already mapped huge page in the boot page table");
            } else {
                pte.paddr() / C::BASE_PAGE_SIZE
            };
            level -= 1;
        }
        // Map the page in the last level page table.
        let index = pte_index::<C>(from, 1);
        let pte_ptr = unsafe { (paddr_to_vaddr(pt * C::BASE_PAGE_SIZE) as *mut E).add(index) };
        let pte = unsafe { pte_ptr.read() };
        if pte.is_present() {
            panic!("mapping an already mapped page in the boot page table");
        }
        unsafe { pte_ptr.write(E::new_frame(to * C::BASE_PAGE_SIZE, 1, prop)) };
    }

    fn alloc_frame(&mut self) -> FrameNumber {
        let frame = FRAME_ALLOCATOR.get().unwrap().lock().alloc(1).unwrap();
        self.frames.push(frame);
        // Zero it out.
        let vaddr = paddr_to_vaddr(frame * PAGE_SIZE) as *mut u8;
        unsafe { core::ptr::write_bytes(vaddr, 0, PAGE_SIZE) };
        frame
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for BootPageTable<E, C> {
    fn drop(&mut self) {
        for frame in &self.frames {
            FRAME_ALLOCATOR.get().unwrap().lock().dealloc(*frame, 1);
        }
    }
}

#[cfg(ktest)]
#[ktest]
fn test_boot_pt() {
    use super::page_walk;
    use crate::{
        arch::mm::{PageTableEntry, PagingConsts},
        vm::{CachePolicy, PageFlags, VmAllocOptions},
    };

    let root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
    let root_paddr = root_frame.start_paddr();

    let mut boot_pt =
        BootPageTable::<PageTableEntry, PagingConsts>::from_anonymous_boot_pt(root_paddr);

    let from1 = 0x1000;
    let to1 = 0x2;
    let prop1 = PageProperty::new(PageFlags::RW, CachePolicy::Writeback);
    boot_pt.map_base_page(from1, to1, prop1);
    assert_eq!(
        unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from1 + 1) },
        Some((to1 * PAGE_SIZE + 1, prop1))
    );

    let from2 = 0x2000;
    let to2 = 0x3;
    let prop2 = PageProperty::new(PageFlags::RX, CachePolicy::Uncacheable);
    boot_pt.map_base_page(from2, to2, prop2);
    assert_eq!(
        unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from2 + 2) },
        Some((to2 * PAGE_SIZE + 2, prop2))
    );
}
