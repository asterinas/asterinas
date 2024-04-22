// SPDX-License-Identifier: MPL-2.0

//! Kernel memory space management.

use align_ext::AlignExt;
use spin::Once;

use super::page_table::PageTableConstsTrait;
use crate::{
    arch::mm::{PageTableConsts, PageTableEntry},
    vm::{
        page_table::{page_walk, CachePolicy, KernelMode, MapProperty, PageTable},
        space::VmPerm,
        MemoryRegionType, Paddr, Vaddr, PAGE_SIZE,
    },
};

/// The base address of the linear mapping of all physical
/// memory in the kernel address space.
pub(crate) const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000;

/// The kernel code is linear mapped to this address.
///
/// FIXME: This offset should be randomly chosen by the loader or the
/// boot compatibility layer. But we disabled it because the framework
/// doesn't support relocatable kernel yet.
pub fn kernel_loaded_offset() -> usize {
    0xffff_ffff_8000_0000
}

pub fn vaddr_to_paddr(va: Vaddr) -> Option<Paddr> {
    if (LINEAR_MAPPING_BASE_VADDR..=kernel_loaded_offset()).contains(&va) {
        // can use offset to get the physical address
        Some(va - LINEAR_MAPPING_BASE_VADDR)
    } else {
        let root_paddr = crate::arch::mm::current_page_table_paddr();
        // Safety: the root page table is valid since we read it from the register.
        unsafe { page_walk::<PageTableEntry, PageTableConsts>(root_paddr, va).map(|(pa, _)| pa) }
    }
}

/// Convert physical address to virtual address using offset, only available inside aster-frame
pub(crate) fn paddr_to_vaddr(pa: Paddr) -> usize {
    pa + LINEAR_MAPPING_BASE_VADDR
}

pub static KERNEL_PAGE_TABLE: Once<PageTable<KernelMode, PageTableEntry, PageTableConsts>> =
    Once::new();

/// Initialize the kernel page table.
///
/// This function should be called after:
///  - the page allocator and the heap allocator are initialized;
///  - the memory regions are initialized.
///
/// This function should be called before:
///  - any initializer that modifies the kernel page table.
pub fn init_kernel_page_table() {
    let kpt = PageTable::<KernelMode>::empty();
    kpt.make_shared_tables(
        PageTableConsts::NR_ENTRIES_PER_FRAME / 2..PageTableConsts::NR_ENTRIES_PER_FRAME,
    );
    let regions = crate::boot::memory_regions();
    // Do linear mappings for the kernel.
    let linear_mapping_size = {
        let mut end = 0;
        for r in regions {
            end = end.max(r.base() + r.len());
        }
        end.align_up(PAGE_SIZE)
    };
    let from = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + linear_mapping_size;
    let to = 0..linear_mapping_size;
    let prop = MapProperty {
        perm: VmPerm::RW,
        global: true,
        extension: 0,
        cache: CachePolicy::Writeback,
    };
    // Safety: we are doing the linear mapping for the kernel.
    unsafe {
        kpt.map_unchecked(&from, &to, prop);
    }
    // Map for the I/O area.
    // TODO: we need to have an allocator to allocate kernel space for
    // the I/O areas, rather than doing it using the linear mappings.
    let to = 0x8_0000_0000..0x9_0000_0000;
    let from = LINEAR_MAPPING_BASE_VADDR + to.start..LINEAR_MAPPING_BASE_VADDR + to.end;
    let prop = MapProperty {
        perm: VmPerm::RW,
        global: true,
        extension: 0,
        cache: CachePolicy::Uncacheable,
    };
    // Safety: we are doing I/O mappings for the kernel.
    unsafe {
        kpt.map_unchecked(&from, &to, prop);
    }
    // Map for the kernel code itself.
    // TODO: set separated permissions for each segments in the kernel.
    let region = regions
        .iter()
        .find(|r| r.typ() == MemoryRegionType::Kernel)
        .unwrap();
    let offset = kernel_loaded_offset();
    let to =
        region.base().align_down(PAGE_SIZE)..(region.base() + region.len()).align_up(PAGE_SIZE);
    let from = to.start + offset..to.end + offset;
    let prop = MapProperty {
        perm: VmPerm::RWX,
        global: true,
        extension: 0,
        cache: CachePolicy::Writeback,
    };
    // Safety: we are doing mappings for the kernel.
    unsafe {
        kpt.map_unchecked(&from, &to, prop);
    }
    KERNEL_PAGE_TABLE.call_once(|| kpt);
}
