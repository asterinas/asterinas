// SPDX-License-Identifier: MPL-2.0

//! Because that the page table implementation requires metadata initialized
//! and mapped, the boot page table is needed to do early stage page table setup
//! in order to initialize the running phase page tables.

use alloc::vec::Vec;

use super::{pte_index, PageTableEntryTrait};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    mm::{
        paddr_to_vaddr, page::allocator::FRAME_ALLOCATOR, PageProperty, PagingConstsTrait, Vaddr,
        PAGE_SIZE,
    },
};

type FrameNumber = usize;

/// A simple boot page table for boot stage mapping management.
/// If applicable, the boot page table could track the lifetime of page table
/// frames that are set up by the firmware, loader or the setup code.
pub struct BootPageTable<
    E: PageTableEntryTrait = PageTableEntry,
    C: PagingConstsTrait = PagingConsts,
> {
    root_pt: FrameNumber,
    // The frames allocated for this page table are not tracked with
    // metadata [`crate::mm::frame::meta`]. Here is a record of it
    // for deallocation.
    frames: Vec<FrameNumber>,
    _pretend_to_use: core::marker::PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> BootPageTable<E, C> {
    /// Creates a new boot page table from the current page table root physical address.
    ///
    /// The caller must ensure that the current page table may be set up by the firmware,
    /// loader or the setup code.
    pub unsafe fn from_current_pt() -> Self {
        let root_paddr = crate::arch::mm::current_page_table_paddr();
        Self {
            root_pt: root_paddr / C::BASE_PAGE_SIZE,
            frames: Vec::new(),
            _pretend_to_use: core::marker::PhantomData,
        }
    }

    /// Maps a base page to a frame.
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

    /// Retires this boot-stage page table.
    ///
    /// Do not drop a boot-stage page table. Instead, retire it.
    ///
    /// # Safety
    ///
    /// This method can only be called when this boot-stage page table is no longer in use,
    /// e.g., after the permanent kernel page table has been activated.
    pub unsafe fn retire(mut self) {
        // Manually free all heap and frame memory allocated.
        let frames = core::mem::take(&mut self.frames);
        for frame in frames {
            FRAME_ALLOCATOR.get().unwrap().lock().dealloc(frame, 1);
        }
        // We do not want or need to trigger drop.
        core::mem::forget(self);
        // FIXME: an empty `Vec` is leaked on the heap here since the drop is not called
        // and we have no ways to free it.
        // The best solution to recycle the boot-phase page table is to initialize all
        // page table page metadata of the boot page table by page walk after the metadata
        // pages are mapped. Therefore the boot page table can be recycled or dropped by
        // the routines in the [`super::node`] module. There's even without a need of
        // `first_activate` concept if the boot page table can be managed by page table
        // pages.
    }
}

impl<E: PageTableEntryTrait, C: PagingConstsTrait> Drop for BootPageTable<E, C> {
    fn drop(&mut self) {
        panic!("the boot page table is dropped rather than retired.");
    }
}

#[cfg(ktest)]
#[ktest]
fn test_boot_pt() {
    use super::page_walk;
    use crate::{
        arch::mm::{PageTableEntry, PagingConsts},
        mm::{CachePolicy, FrameAllocOptions, PageFlags},
    };

    let root_frame = FrameAllocOptions::new(1).alloc_single().unwrap();
    let root_paddr = root_frame.start_paddr();

    let mut boot_pt = BootPageTable::<PageTableEntry, PagingConsts> {
        root_pt: root_paddr / PagingConsts::BASE_PAGE_SIZE,
        frames: Vec::new(),
        _pretend_to_use: core::marker::PhantomData,
    };

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

    unsafe { boot_pt.retire() }
}
