// SPDX-License-Identifier: MPL-2.0

//! Kernel memory space management.
//!
//! The kernel memory space is currently managed as follows, if the
//! address width is 48 bits (with 47 bits kernel space).
//!
//! TODO: the cap of linear mapping (the start of vm alloc) are raised
//! to workaround for high IO in TDX. We need actual vm alloc API to have
//! a proper fix.
//!
//! ```text
//! +-+ <- the highest used address (0xffff_ffff_ffff_0000)
//! | |         For the kernel code, 1 GiB.
//! +-+ <- 0xffff_ffff_8000_0000
//! | |
//! | |         Unused hole.
//! +-+ <- 0xffff_e100_0000_0000
//! | |         For frame metadata, 1 TiB.
//! +-+ <- 0xffff_e000_0000_0000
//! | |         For [`KVirtArea`], 32 TiB.
//! +-+ <- the middle of the higher half (0xffff_c000_0000_0000)
//! | |
//! | |
//! | |
//! | |         For linear mappings, 64 TiB.
//! | |         Mapped physical addresses are untracked.
//! | |
//! | |
//! | |
//! +-+ <- the base of high canonical address (0xffff_8000_0000_0000)
//! ```
//!
//! If the address width is (according to [`crate::arch::mm::PagingConsts`])
//! 39 bits or 57 bits, the memory space just adjust proportionally.

pub(crate) mod kvirt_area;

use core::ops::Range;

use log::info;
use spin::Once;
#[cfg(ktest)]
mod test;

use super::{
    frame::{
        meta::{mapping, AnyFrameMeta, MetaPageMeta},
        Segment,
    },
    page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    page_table::{PageTable, PageTableConfig},
    Frame, Paddr, PagingConstsTrait, Vaddr,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    boot::memory_region::MemoryRegionType,
    mm::{page_table::largest_pages, PagingLevel},
    task::disable_preempt,
};

/// The shortest supported address width is 39 bits. And the literal
/// values are written for 48 bits address width. Adjust the values
/// by arithmetic left shift.
const ADDR_WIDTH_SHIFT: isize = PagingConsts::ADDRESS_WIDTH as isize - 48;

/// Start of the kernel address space.
/// This is the _lowest_ address of the x86-64's _high_ canonical addresses.
#[cfg(not(target_arch = "loongarch64"))]
pub const KERNEL_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000 << ADDR_WIDTH_SHIFT;
#[cfg(target_arch = "loongarch64")]
pub const KERNEL_BASE_VADDR: Vaddr = 0x9000_0000_0000_0000 << ADDR_WIDTH_SHIFT;
/// End of the kernel address space (non inclusive).
pub const KERNEL_END_VADDR: Vaddr = 0xffff_ffff_ffff_0000 << ADDR_WIDTH_SHIFT;

/// The kernel code is linear mapped to this address.
///
/// FIXME: This offset should be randomly chosen by the loader or the
/// boot compatibility layer. But we disabled it because OSTD
/// doesn't support relocatable kernel yet.
pub fn kernel_loaded_offset() -> usize {
    KERNEL_CODE_BASE_VADDR
}

#[cfg(target_arch = "x86_64")]
const KERNEL_CODE_BASE_VADDR: usize = 0xffff_ffff_8000_0000 << ADDR_WIDTH_SHIFT;
#[cfg(target_arch = "riscv64")]
const KERNEL_CODE_BASE_VADDR: usize = 0xffff_ffff_0000_0000 << ADDR_WIDTH_SHIFT;
#[cfg(target_arch = "loongarch64")]
const KERNEL_CODE_BASE_VADDR: usize = 0x9000_0000_0000_0000 << ADDR_WIDTH_SHIFT;

const FRAME_METADATA_CAP_VADDR: Vaddr = 0xffff_e100_0000_0000 << ADDR_WIDTH_SHIFT;
const FRAME_METADATA_BASE_VADDR: Vaddr = 0xffff_e000_0000_0000 << ADDR_WIDTH_SHIFT;
pub(in crate::mm) const FRAME_METADATA_RANGE: Range<Vaddr> =
    FRAME_METADATA_BASE_VADDR..FRAME_METADATA_CAP_VADDR;

const VMALLOC_BASE_VADDR: Vaddr = 0xffff_c000_0000_0000 << ADDR_WIDTH_SHIFT;
pub const VMALLOC_VADDR_RANGE: Range<Vaddr> = VMALLOC_BASE_VADDR..FRAME_METADATA_BASE_VADDR;

/// The base address of the linear mapping of all physical
/// memory in the kernel address space.
#[cfg(not(target_arch = "loongarch64"))]
pub const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000 << ADDR_WIDTH_SHIFT;
#[cfg(target_arch = "loongarch64")]
pub const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0x9000_0000_0000_0000 << ADDR_WIDTH_SHIFT;
pub const LINEAR_MAPPING_VADDR_RANGE: Range<Vaddr> = LINEAR_MAPPING_BASE_VADDR..VMALLOC_BASE_VADDR;

/// Convert physical address to virtual address using offset, only available inside `ostd`
pub fn paddr_to_vaddr(pa: Paddr) -> usize {
    debug_assert!(pa < VMALLOC_BASE_VADDR - LINEAR_MAPPING_BASE_VADDR);
    pa + LINEAR_MAPPING_BASE_VADDR
}

/// The kernel page table instance.
///
/// It manages the kernel mapping of all address spaces by sharing the kernel part. And it
/// is unlikely to be activated.
pub static KERNEL_PAGE_TABLE: Once<PageTable<KernelPtConfig>> = Once::new();

#[derive(Clone, Debug)]
pub(crate) struct KernelPtConfig {}

// We use the first available PTE bit to mark the frame as tracked.
// SAFETY: `item_into_raw` and `item_from_raw` are implemented correctly,
unsafe impl PageTableConfig for KernelPtConfig {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 256..512;
    const TOP_LEVEL_CAN_UNMAP: bool = false;

    type E = PageTableEntry;
    type C = PagingConsts;

    type Item = MappedItem;

    fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        match item {
            MappedItem::Tracked(frame, mut prop) => {
                debug_assert!(!prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1));
                prop.priv_flags |= PrivilegedPageFlags::AVAIL1;
                let level = frame.map_level();
                let paddr = frame.into_raw();
                (paddr, level, prop)
            }
            MappedItem::Untracked(pa, level, mut prop) => {
                debug_assert!(!prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1));
                prop.priv_flags -= PrivilegedPageFlags::AVAIL1;
                (pa, level, prop)
            }
        }
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        if prop.priv_flags.contains(PrivilegedPageFlags::AVAIL1) {
            debug_assert_eq!(level, 1);
            // SAFETY: The caller ensures safety.
            let frame = unsafe { Frame::<dyn AnyFrameMeta>::from_raw(paddr) };
            MappedItem::Tracked(frame, prop)
        } else {
            MappedItem::Untracked(paddr, level, prop)
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum MappedItem {
    Tracked(Frame<dyn AnyFrameMeta>, PageProperty),
    Untracked(Paddr, PagingLevel, PageProperty),
}

/// Initializes the kernel page table.
///
/// This function should be called after:
///  - the page allocator and the heap allocator are initialized;
///  - the memory regions are initialized.
///
/// This function should be called before:
///  - any initializer that modifies the kernel page table.
pub fn init_kernel_page_table(meta_pages: Segment<MetaPageMeta>) {
    info!("Initializing the kernel page table");

    // Start to initialize the kernel page table.
    let kpt = PageTable::<KernelPtConfig>::new_kernel_page_table();
    let preempt_guard = disable_preempt();

    // In LoongArch64, we don't need to do linear mappings for the kernel because of DMW0.
    #[cfg(not(target_arch = "loongarch64"))]
    // Do linear mappings for the kernel.
    {
        let max_paddr = crate::mm::frame::max_paddr();
        let from = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + max_paddr;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut cursor = kpt.cursor_mut(&preempt_guard, &from).unwrap();
        for (pa, level) in largest_pages::<KernelPtConfig>(from.start, 0, max_paddr) {
            // SAFETY: we are doing the linear mapping for the kernel.
            unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) }
                .expect("Kernel linear address space is mapped twice");
        }
    }

    // Map the metadata pages.
    {
        let start_va = mapping::frame_to_meta::<PagingConsts>(0);
        let from = start_va..start_va + meta_pages.size();
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut cursor = kpt.cursor_mut(&preempt_guard, &from).unwrap();
        // We use untracked mapping so that we can benefit from huge pages.
        // We won't unmap them anyway, so there's no leaking problem yet.
        // TODO: support tracked huge page mapping.
        let pa_range = meta_pages.into_raw();
        for (pa, level) in
            largest_pages::<KernelPtConfig>(from.start, pa_range.start, pa_range.len())
        {
            // SAFETY: We are doing the metadata mappings for the kernel.
            unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) }
                .expect("Frame metadata address space is mapped twice");
        }
    }

    // In LoongArch64, we don't need to do linear mappings for the kernel code because of DMW0.
    #[cfg(not(target_arch = "loongarch64"))]
    // Map for the kernel code itself.
    // TODO: set separated permissions for each segments in the kernel.
    {
        let regions = &crate::boot::EARLY_INFO.get().unwrap().memory_regions;
        let region = regions
            .iter()
            .find(|r| r.typ() == MemoryRegionType::Kernel)
            .unwrap();
        let offset = kernel_loaded_offset();
        let from = region.base() + offset..region.end() + offset;
        let prop = PageProperty {
            flags: PageFlags::RWX,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut cursor = kpt.cursor_mut(&preempt_guard, &from).unwrap();
        for (pa, level) in largest_pages::<KernelPtConfig>(from.start, region.base(), from.len()) {
            // SAFETY: we are doing the kernel code mapping.
            unsafe { cursor.map(MappedItem::Untracked(pa, level, prop)) }
                .expect("Kernel code mapped twice");
        }
    }

    KERNEL_PAGE_TABLE.call_once(|| kpt);
}

/// Activates the kernel page table.
///
/// # Safety
///
/// This function should only be called once per CPU.
pub unsafe fn activate_kernel_page_table() {
    let kpt = KERNEL_PAGE_TABLE
        .get()
        .expect("The kernel page table is not initialized yet");
    // SAFETY: the kernel page table is initialized properly.
    unsafe {
        kpt.first_activate_unchecked();
        crate::arch::mm::tlb_flush_all_including_global();
    }

    // SAFETY: the boot page table is OK to be dismissed now since
    // the kernel page table is activated just now.
    unsafe {
        crate::mm::page_table::boot_pt::dismiss();
    }
}
