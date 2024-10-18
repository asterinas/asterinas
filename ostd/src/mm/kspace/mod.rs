// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]

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
//! | |         For the kernel code, 1 GiB. Mapped frames are tracked.
//! +-+ <- 0xffff_ffff_8000_0000
//! | |
//! | |         Unused hole.
//! +-+ <- 0xffff_e100_0000_0000
//! | |         For frame metadata, 1 TiB. Mapped frames are untracked.
//! +-+ <- 0xffff_e000_0000_0000
//! | |         For [`KVirtArea<Tracked>`], 16 TiB. Mapped pages are tracked with handles.
//! +-+ <- 0xffff_d000_0000_0000
//! | |         For [`KVirtArea<Untracked>`], 16 TiB. Mapped pages are untracked.
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

use alloc::vec::Vec;
use core::ops::Range;

use align_ext::AlignExt;
use log::info;
use spin::Once;

use super::{
    nr_subpage_per_huge,
    page::{
        meta::{mapping, KernelMeta, MetaPageMeta},
        Page,
    },
    page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags},
    page_table::{KernelMode, PageTable},
    Paddr, PagingConstsTrait, Vaddr, PAGE_SIZE,
};
use crate::{
    arch::mm::{PageTableEntry, PagingConsts},
    boot::memory_region::MemoryRegionType,
};

/// The shortest supported address width is 39 bits. And the literal
/// values are written for 48 bits address width. Adjust the values
/// by arithmetic left shift.
const ADDR_WIDTH_SHIFT: isize = PagingConsts::ADDRESS_WIDTH as isize - 48;

/// Start of the kernel address space.
/// This is the _lowest_ address of the x86-64's _high_ canonical addresses.
pub const KERNEL_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000 << ADDR_WIDTH_SHIFT;
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

const FRAME_METADATA_CAP_VADDR: Vaddr = 0xffff_e100_0000_0000 << ADDR_WIDTH_SHIFT;
const FRAME_METADATA_BASE_VADDR: Vaddr = 0xffff_e000_0000_0000 << ADDR_WIDTH_SHIFT;
pub(in crate::mm) const FRAME_METADATA_RANGE: Range<Vaddr> =
    FRAME_METADATA_BASE_VADDR..FRAME_METADATA_CAP_VADDR;

const TRACKED_MAPPED_PAGES_BASE_VADDR: Vaddr = 0xffff_d000_0000_0000 << ADDR_WIDTH_SHIFT;
pub const TRACKED_MAPPED_PAGES_RANGE: Range<Vaddr> =
    TRACKED_MAPPED_PAGES_BASE_VADDR..FRAME_METADATA_BASE_VADDR;

const VMALLOC_BASE_VADDR: Vaddr = 0xffff_c000_0000_0000 << ADDR_WIDTH_SHIFT;
pub const VMALLOC_VADDR_RANGE: Range<Vaddr> = VMALLOC_BASE_VADDR..TRACKED_MAPPED_PAGES_BASE_VADDR;

/// The base address of the linear mapping of all physical
/// memory in the kernel address space.
pub const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000 << ADDR_WIDTH_SHIFT;
pub const LINEAR_MAPPING_VADDR_RANGE: Range<Vaddr> = LINEAR_MAPPING_BASE_VADDR..VMALLOC_BASE_VADDR;

/// Convert physical address to virtual address using offset, only available inside `ostd`
pub fn paddr_to_vaddr(pa: Paddr) -> usize {
    debug_assert!(pa < VMALLOC_BASE_VADDR - LINEAR_MAPPING_BASE_VADDR);
    pa + LINEAR_MAPPING_BASE_VADDR
}

/// Returns whether the given address should be mapped as tracked.
///
/// About what is tracked mapping, see [`crate::mm::page::meta::MapTrackingStatus`].
pub(crate) fn should_map_as_tracked(addr: Vaddr) -> bool {
    !(LINEAR_MAPPING_VADDR_RANGE.contains(&addr) || VMALLOC_VADDR_RANGE.contains(&addr))
}

/// The kernel page table instance.
///
/// It manages the kernel mapping of all address spaces by sharing the kernel part. And it
/// is unlikely to be activated.
pub static KERNEL_PAGE_TABLE: Once<PageTable<KernelMode, PageTableEntry, PagingConsts>> =
    Once::new();

/// Initializes the kernel page table.
///
/// This function should be called after:
///  - the page allocator and the heap allocator are initialized;
///  - the memory regions are initialized.
///
/// This function should be called before:
///  - any initializer that modifies the kernel page table.
pub fn init_kernel_page_table(meta_pages: Vec<Page<MetaPageMeta>>) {
    info!("Initializing the kernel page table");

    let regions = crate::boot::memory_regions();
    let phys_mem_cap = regions.iter().map(|r| r.base() + r.len()).max().unwrap();

    // Start to initialize the kernel page table.
    let kpt = PageTable::<KernelMode>::empty();

    // Make shared the page tables mapped by the root table in the kernel space.
    {
        let pte_index_max = nr_subpage_per_huge::<PagingConsts>();
        kpt.make_shared_tables(pte_index_max / 2..pte_index_max);
    }

    // Do linear mappings for the kernel.
    {
        let from = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + phys_mem_cap;
        let to = 0..phys_mem_cap;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        // SAFETY: we are doing the linear mapping for the kernel.
        unsafe {
            kpt.map(&from, &to, prop).unwrap();
        }
    }

    // Map the metadata pages.
    {
        let start_va = mapping::page_to_meta::<PagingConsts>(0);
        let from = start_va..start_va + meta_pages.len() * PAGE_SIZE;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut cursor = kpt.cursor_mut(&from).unwrap();
        for meta_page in meta_pages {
            // SAFETY: we are doing the metadata mappings for the kernel.
            unsafe {
                let _old = cursor.map(meta_page.into(), prop);
            }
        }
    }

    // Map for the I/O area.
    // TODO: we need to have an allocator to allocate kernel space for
    // the I/O areas, rather than doing it using the linear mappings.
    {
        let to = 0x8_0000_0000..0x9_0000_0000;
        let from = LINEAR_MAPPING_BASE_VADDR + to.start..LINEAR_MAPPING_BASE_VADDR + to.end;
        let prop = PageProperty {
            flags: PageFlags::RW,
            cache: CachePolicy::Uncacheable,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        // SAFETY: we are doing I/O mappings for the kernel.
        unsafe {
            kpt.map(&from, &to, prop).unwrap();
        }
    }

    // Map for the kernel code itself.
    // TODO: set separated permissions for each segments in the kernel.
    {
        let region = regions
            .iter()
            .find(|r| r.typ() == MemoryRegionType::Kernel)
            .unwrap();
        let offset = kernel_loaded_offset();
        let to =
            region.base().align_down(PAGE_SIZE)..(region.base() + region.len()).align_up(PAGE_SIZE);
        let from = to.start + offset..to.end + offset;
        let prop = PageProperty {
            flags: PageFlags::RWX,
            cache: CachePolicy::Writeback,
            priv_flags: PrivilegedPageFlags::GLOBAL,
        };
        let mut cursor = kpt.cursor_mut(&from).unwrap();
        for frame_paddr in to.step_by(PAGE_SIZE) {
            let page = Page::<KernelMeta>::from_unused(frame_paddr, KernelMeta::default());
            // SAFETY: we are doing mappings for the kernel.
            unsafe {
                let _old = cursor.map(page.into(), prop);
            }
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
