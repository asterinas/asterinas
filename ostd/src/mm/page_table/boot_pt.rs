// SPDX-License-Identifier: MPL-2.0

//! Because that the page table implementation requires metadata initialized
//! and mapped, the boot page table is needed to do early stage page table setup
//! in order to initialize the running phase page tables.

use alloc::vec::Vec;
use core::{
    result::Result,
    sync::atomic::{AtomicU32, Ordering},
};

use super::{pte_index, PageTableEntryTrait};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    cpu::num_cpus,
    cpu_local_cell,
    mm::{
        nr_subpage_per_huge, paddr_to_vaddr, page::allocator::PAGE_ALLOCATOR, PageProperty,
        PagingConstsTrait, Vaddr, PAGE_SIZE,
    },
    sync::SpinLock,
};

type FrameNumber = usize;

/// The accessor to the boot page table singleton [`BootPageTable`].
///
/// The user should provide a closure to access the boot page table. The
/// function will acquire the lock and call the closure with a mutable
/// reference to the boot page table as the argument.
///
/// The boot page table will be dropped when there's no CPU activating it.
/// This function will return an [`Err`] if the boot page table is dropped.
pub(crate) fn with_borrow<F>(f: F) -> Result<(), ()>
where
    F: FnOnce(&mut BootPageTable),
{
    let mut boot_pt = BOOT_PAGE_TABLE.lock();

    if IS_DISMISSED.load() {
        return Err(());
    }

    // Lazy initialization.
    if boot_pt.is_none() {
        // SAFETY: This function is called only once.
        *boot_pt = Some(unsafe { BootPageTable::from_current_pt() });
    }

    f(boot_pt.as_mut().unwrap());

    Ok(())
}

/// Dismiss the boot page table.
///
/// By calling it on a CPU, the caller claims that the boot page table is no
/// longer needed on this CPU.
///
/// # Safety
///
/// The caller should ensure that:
///  - another legitimate page table is activated on this CPU;
///  - this function should be called only once per CPU;
///  - no [`with`] calls are performed on this CPU after this dismissal;
///  - no [`with`] calls are performed on this CPU after the activation of
///    another page table and before this dismissal.
pub(crate) unsafe fn dismiss() {
    IS_DISMISSED.store(true);
    if DISMISS_COUNT.fetch_add(1, Ordering::SeqCst) as usize == num_cpus() - 1 {
        BOOT_PAGE_TABLE.lock().take();
    }
}

/// The boot page table singleton instance.
static BOOT_PAGE_TABLE: SpinLock<Option<BootPageTable>> = SpinLock::new(None);
/// If it reaches the number of CPUs, the boot page table will be dropped.
static DISMISS_COUNT: AtomicU32 = AtomicU32::new(0);
cpu_local_cell! {
    /// If the boot page table is dismissed on this CPU.
    static IS_DISMISSED: bool = false;
}

/// A simple boot page table singleton for boot stage mapping management.
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
    /// Creates a new boot page table from the current page table root
    /// physical address.
    ///
    /// # Safety
    ///
    /// This function should be called only once in the initialization phase.
    /// Otherwise, It would lead to double-drop of the page table frames set up
    /// by the firmware, loader or the setup code.
    unsafe fn from_current_pt() -> Self {
        let root_paddr = crate::arch::mm::current_page_table_paddr();
        Self {
            root_pt: root_paddr / C::BASE_PAGE_SIZE,
            frames: Vec::new(),
            _pretend_to_use: core::marker::PhantomData,
        }
    }

    /// Maps a base page to a frame.
    ///
    /// # Panics
    ///
    /// This function will panic if the page is already mapped.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it can cause undefined behavior if the caller
    /// maps a page in the kernel address space.
    pub unsafe fn map_base_page(&mut self, from: Vaddr, to: FrameNumber, prop: PageProperty) {
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
        unsafe { pte_ptr.write(E::new_page(to * C::BASE_PAGE_SIZE, 1, prop)) };
    }

    /// Set protections of a base page mapping.
    ///
    /// This function may split a huge page into base pages, causing page allocations
    /// if the original mapping is a huge page.
    ///
    /// # Panics
    ///
    /// This function will panic if the page is already mapped.
    ///
    /// # Safety
    ///
    /// This function is unsafe because it can cause undefined behavior if the caller
    /// maps a page in the kernel address space.
    pub unsafe fn protect_base_page(
        &mut self,
        virt_addr: Vaddr,
        mut op: impl FnMut(&mut PageProperty),
    ) {
        let mut pt = self.root_pt;
        let mut level = C::NR_LEVELS;
        // Walk to the last level of the page table.
        while level > 1 {
            let index = pte_index::<C>(virt_addr, level);
            let pte_ptr = unsafe { (paddr_to_vaddr(pt * C::BASE_PAGE_SIZE) as *mut E).add(index) };
            let pte = unsafe { pte_ptr.read() };
            pt = if !pte.is_present() {
                panic!("protecting an unmapped page in the boot page table");
            } else if pte.is_last(level) {
                // Split the huge page.
                let frame = self.alloc_frame();
                let huge_pa = pte.paddr();
                for i in 0..nr_subpage_per_huge::<C>() {
                    let nxt_ptr =
                        unsafe { (paddr_to_vaddr(frame * C::BASE_PAGE_SIZE) as *mut E).add(i) };
                    unsafe {
                        nxt_ptr.write(E::new_page(
                            huge_pa + i * C::BASE_PAGE_SIZE,
                            level - 1,
                            pte.prop(),
                        ))
                    };
                }
                unsafe { pte_ptr.write(E::new_pt(frame * C::BASE_PAGE_SIZE)) };
                frame
            } else {
                pte.paddr() / C::BASE_PAGE_SIZE
            };
            level -= 1;
        }
        // Do protection in the last level page table.
        let index = pte_index::<C>(virt_addr, 1);
        let pte_ptr = unsafe { (paddr_to_vaddr(pt * C::BASE_PAGE_SIZE) as *mut E).add(index) };
        let pte = unsafe { pte_ptr.read() };
        if !pte.is_present() {
            panic!("protecting an unmapped page in the boot page table");
        }
        let mut prop = pte.prop();
        op(&mut prop);
        unsafe { pte_ptr.write(E::new_page(pte.paddr(), 1, prop)) };
    }

    fn alloc_frame(&mut self) -> FrameNumber {
        let frame = PAGE_ALLOCATOR.get().unwrap().lock().alloc(1).unwrap();
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
            PAGE_ALLOCATOR.get().unwrap().lock().dealloc(*frame, 1);
        }
    }
}

#[cfg(ktest)]
use crate::prelude::*;

#[cfg(ktest)]
#[ktest]
fn test_boot_pt_map_protect() {
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
    unsafe { boot_pt.map_base_page(from1, to1, prop1) };
    assert_eq!(
        unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from1 + 1) },
        Some((to1 * PAGE_SIZE + 1, prop1))
    );
    unsafe { boot_pt.protect_base_page(from1, |prop| prop.flags = PageFlags::RX) };
    assert_eq!(
        unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from1 + 1) },
        Some((
            to1 * PAGE_SIZE + 1,
            PageProperty::new(PageFlags::RX, CachePolicy::Writeback)
        ))
    );

    let from2 = 0x2000;
    let to2 = 0x3;
    let prop2 = PageProperty::new(PageFlags::RX, CachePolicy::Uncacheable);
    unsafe { boot_pt.map_base_page(from2, to2, prop2) };
    assert_eq!(
        unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from2 + 2) },
        Some((to2 * PAGE_SIZE + 2, prop2))
    );
    unsafe { boot_pt.protect_base_page(from2, |prop| prop.flags = PageFlags::RW) };
    assert_eq!(
        unsafe { page_walk::<PageTableEntry, PagingConsts>(root_paddr, from2 + 2) },
        Some((
            to2 * PAGE_SIZE + 2,
            PageProperty::new(PageFlags::RW, CachePolicy::Uncacheable)
        ))
    );
}
