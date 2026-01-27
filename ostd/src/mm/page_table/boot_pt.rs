// SPDX-License-Identifier: MPL-2.0

//! Because that the page table implementation requires metadata initialized
//! and mapped, the boot page table is needed to do early stage page table setup
//! in order to initialize the running phase page tables.

use core::{
    alloc::Layout,
    result::Result,
    sync::atomic::{AtomicU32, Ordering},
};

use ostd_pod::FromZeros;

use super::{PteTrait, pte_index};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    cpu::num_cpus,
    cpu_local_cell,
    mm::{
        Frame, FrameAllocOptions, PAGE_SIZE, Paddr, PageProperty, PagingConstsTrait, PagingLevel,
        Vaddr,
        frame::{
            self,
            allocator::{self, EarlyAllocatedFrameMeta},
        },
        nr_subpage_per_huge, paddr_to_vaddr,
        page_prop::PageTableFlags,
        page_table::PteScalar,
    },
    sync::SpinLock,
};

/// The accessor to the boot page table singleton [`BootPageTable`].
///
/// The user should provide a closure to access the boot page table. The
/// function will acquire the lock and call the closure with a mutable
/// reference to the boot page table as the argument.
///
/// The boot page table will be dropped when all CPUs have dismissed it.
/// This function will return an [`Err`] if the boot page table is dropped.
pub(crate) fn with_borrow<F, R>(f: F) -> Result<R, ()>
where
    F: FnOnce(&mut BootPageTable) -> R,
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

    let r = f(boot_pt.as_mut().unwrap());

    Ok(r)
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
///  - no [`with_borrow`] calls are performed on this CPU after this dismissal;
pub(crate) unsafe fn dismiss() {
    IS_DISMISSED.store(true);
    if DISMISS_COUNT.fetch_add(1, Ordering::SeqCst) as usize == num_cpus() - 1 {
        let boot_pt = BOOT_PAGE_TABLE.lock().take().unwrap();

        dfs_walk_on_leave::<PageTableEntry, PagingConsts>(
            boot_pt.root_pt,
            PagingConsts::NR_LEVELS,
            &mut |pte, pa, _, flags| {
                if !flags.contains(PTE_POINTS_TO_FIRMWARE_PT) {
                    // SAFETY: The pointed frame is allocated and forgotten with `into_raw`.
                    drop(unsafe { Frame::<EarlyAllocatedFrameMeta>::from_raw(pa) })
                }
                // Firmware provided page tables may be a DAG instead of a tree.
                // Clear it to avoid double-free when we meet it the second time.
                *pte = PageTableEntry::new_zeroed();
            },
        );
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
///
/// If applicable, the boot page table could track the lifetime of page table
/// frames that are set up by the firmware, loader or the setup code.
///
/// All the newly allocated page table frames have the first unused bit in
/// parent PTEs. This allows us to deallocate them when the boot page table
/// is dropped.
pub(crate) struct BootPageTable<E: PteTrait = PageTableEntry, C: PagingConstsTrait = PagingConsts> {
    root_pt: Paddr,
    _phantom: core::marker::PhantomData<(E, C)>,
}

// We use extra two available bits in the boot PT for memory management.
//
// The first available bit is used to differentiate firmware page tables from
// the page tables allocated here. The second is for identifying double-visits
// when walking the page tables since the PT can be a DAG.
const PTE_POINTS_TO_FIRMWARE_PT: PageTableFlags = PageTableFlags::AVAIL1;

impl<E: PteTrait, C: PagingConstsTrait> BootPageTable<E, C> {
    /// Creates a new boot page table from the current page table root
    /// physical address.
    ///
    /// # Safety
    ///
    /// This function should be called only once in the initialization phase.
    /// Otherwise, It would lead to double-drop of the page table frames set up
    /// by the firmware, loader or the setup code.
    unsafe fn from_current_pt() -> Self {
        let root_pt = crate::arch::mm::current_page_table_paddr();
        // Make sure the 2 available bits are not set for firmware page tables.
        dfs_walk_on_leave::<E, C>(
            root_pt,
            C::NR_LEVELS,
            &mut |pte: &mut E, pa, level, mut flags| {
                flags |= PTE_POINTS_TO_FIRMWARE_PT;
                *pte = E::from_repr(&PteScalar::PageTable(pa, flags), level);
            },
        );
        Self {
            root_pt,
            _phantom: core::marker::PhantomData,
        }
    }

    /// Returns the root physical address of the boot page table.
    pub(crate) fn root_address(&self) -> Paddr {
        self.root_pt
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
    pub unsafe fn map_base_page(&mut self, from: Vaddr, to: Paddr, prop: PageProperty) {
        let mut pt = self.root_pt;
        let mut level = C::NR_LEVELS;
        // Walk to the last level of the page table.
        while level > 1 {
            let index = pte_index::<C>(from, level);
            // SAFETY: The result pointer is within the PT frame.
            let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
            // SAFETY: The pointer to the entry is valid to read.
            let pte = unsafe { pte_ptr.read() };
            match pte.to_repr(level) {
                PteScalar::Absent => {
                    let (pte, child_pt) = self.alloc_child(level);
                    // SAFETY: The pointer to the entry is valid to write.
                    unsafe { pte_ptr.write(pte) };
                    pt = child_pt;
                }
                PteScalar::Mapped(_, _) => {
                    panic!("mapping an already mapped huge page in the boot page table");
                }
                PteScalar::PageTable(child_pt, _) => {
                    pt = child_pt;
                }
            };
            level -= 1;
        }
        // Map the page in the last level page table.
        let index = pte_index::<C>(from, 1);
        // SAFETY: The result pointer is within the PT frame.
        let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
        // SAFETY: The pointer to the entry is valid to read.
        let pte = unsafe { pte_ptr.read() };
        if matches!(pte.to_repr(1), PteScalar::Mapped(_, _)) {
            panic!("mapping an already mapped page in the boot page table");
        }
        // SAFETY: The pointer to the entry is valid to write.
        unsafe { pte_ptr.write(E::from_repr(&PteScalar::Mapped(to, prop), 1)) };
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
    #[cfg_attr(not(ktest), expect(dead_code))]
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
            // SAFETY: The result pointer is within the PT frame.
            let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
            // SAFETY: The pointer to the entry is valid to read.
            let pte = unsafe { pte_ptr.read() };
            match pte.to_repr(level) {
                PteScalar::Absent => {
                    panic!("protecting an unmapped page in the boot page table");
                }
                PteScalar::PageTable(child_pt, _) => {
                    pt = child_pt;
                }
                PteScalar::Mapped(huge_pa, prop) => {
                    // Split the huge page.
                    let (child_pte, child_frame_pa) = self.alloc_child(level);
                    for i in 0..nr_subpage_per_huge::<C>() {
                        // SAFETY: The result pointer is within the new PT frame.
                        let nxt_ptr = unsafe { (paddr_to_vaddr(child_frame_pa) as *mut E).add(i) };
                        // SAFETY: The pointer to the entry is valid to write.
                        unsafe {
                            nxt_ptr.write(E::from_repr(
                                &PteScalar::Mapped(huge_pa + i * C::BASE_PAGE_SIZE, prop),
                                level - 1,
                            ))
                        };
                    }
                    // SAFETY: The pointer to the entry is valid to write.
                    unsafe { pte_ptr.write(child_pte) };
                    pt = child_frame_pa;
                }
            };
            level -= 1;
        }
        // Do protection in the last level page table.
        let index = pte_index::<C>(virt_addr, 1);
        // SAFETY: The result pointer is within the PT frame.
        let pte_ptr = unsafe { (paddr_to_vaddr(pt) as *mut E).add(index) };
        // SAFETY: The pointer to the entry is valid to read.
        let pte = unsafe { pte_ptr.read() };
        let PteScalar::Mapped(pa, mut prop) = pte.to_repr(1) else {
            panic!("protecting an unmapped page in the boot page table");
        };
        op(&mut prop);
        // SAFETY: The pointer to the entry is valid to write.
        unsafe { pte_ptr.write(E::from_repr(&PteScalar::Mapped(pa, prop), 1)) };
    }

    fn alloc_child(&mut self, level: PagingLevel) -> (E, Paddr) {
        let frame_paddr = if frame::meta::is_initialized() {
            let frame = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_frame_with(EarlyAllocatedFrameMeta)
                .unwrap();
            frame.into_raw()
        } else {
            allocator::early_alloc(
                Layout::from_size_align(C::BASE_PAGE_SIZE, C::BASE_PAGE_SIZE).unwrap(),
            )
            .unwrap()
        };

        // Zero it out.
        let vaddr = paddr_to_vaddr(frame_paddr) as *mut u8;
        // SAFETY: The allocated frame is valid to write.
        unsafe { core::ptr::write_bytes(vaddr, 0, PAGE_SIZE) };

        (
            E::from_repr(
                &PteScalar::PageTable(frame_paddr, PageTableFlags::empty()),
                level,
            ),
            frame_paddr,
        )
    }

    #[cfg(ktest)]
    pub(super) fn new(root_pt: Paddr) -> Self {
        Self {
            root_pt,
            _phantom: core::marker::PhantomData,
        }
    }
}

/// A helper function to walk on the page table frames.
///
/// Once leaving a page table frame, the closure will be called with the PTE to
/// the frame.
fn dfs_walk_on_leave<E: PteTrait, C: PagingConstsTrait>(
    pt: Paddr,
    level: PagingLevel,
    op: &mut impl FnMut(&mut E, Paddr, PagingLevel, PageTableFlags),
) {
    if level >= 2 {
        let pt_vaddr = paddr_to_vaddr(pt) as *mut E;
        for offset in 0..nr_subpage_per_huge::<C>() {
            // SAFETY: The result pointer is within the PT frame.
            let pte_ptr = unsafe { pt_vaddr.add(offset) };
            // SAFETY: The pointer to the entry is valid to read.
            let mut pte = unsafe { pte_ptr.read() };

            if let PteScalar::PageTable(child_pt, flags) = pte.to_repr(level) {
                dfs_walk_on_leave::<E, C>(child_pt, level - 1, op);
                op(&mut pte, child_pt, level, flags);
                // SAFETY: The pointer to the entry is valid to write.
                unsafe { pte_ptr.write(pte) };
            }
        }
    }
}
