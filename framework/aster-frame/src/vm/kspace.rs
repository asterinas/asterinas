// SPDX-License-Identifier: MPL-2.0

//! Kernel memory space management.

use core::ops::Range;

use align_ext::AlignExt;
use spin::Once;
use static_assertions::const_assert;

use super::{
    page_table::{nr_ptes_per_node, KernelMode, PageTable},
    CachePolicy, MemoryRegionType, Paddr, PageFlags, PageProperty, PrivilegedPageFlags, Vaddr,
    PAGE_SIZE,
};
use crate::arch::mm::{PageTableEntry, PagingConsts};

/// The base address of the linear mapping of all physical
/// memory in the kernel address space.
pub(crate) const LINEAR_MAPPING_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000;

/// The maximum size of the direct mapping of physical memory.
///
/// This size acts as a cap. If the actual memory size exceeds this value,
/// the remaining memory cannot be included in the direct mapping because
/// the maximum size of the direct mapping is limited by this value. On
/// the other hand, if the actual memory size is smaller, the direct
/// mapping can shrink to save memory consumption due to the page table.
///
/// We do not currently have APIs to manually map MMIO pages, so we have
/// to rely on the direct mapping to perform MMIO operations. Therefore,
/// we set the maximum size to 127 TiB, which makes some surprisingly
/// high MMIO addresses usable (e.g., `0x7000_0000_7004` for VirtIO
/// devices in the TDX environment) and leaves the last 1 TiB for other
/// uses (e.g., the kernel code starting at [`kernel_loaded_offset()`]).
pub(crate) const LINEAR_MAPPING_MAX_SIZE: usize = 127 << 40;

/// The address range of the direct mapping of physical memory.
///
/// This range is constructed based on [`PHYS_MEM_BASE_VADDR`] and
/// [`PHYS_MEM_MAPPING_MAX_SIZE`].
pub(crate) const LINEAR_MAPPING_VADDR_RANGE: Range<Vaddr> =
    LINEAR_MAPPING_BASE_VADDR..(LINEAR_MAPPING_BASE_VADDR + LINEAR_MAPPING_MAX_SIZE);

/// The kernel code is linear mapped to this address.
///
/// FIXME: This offset should be randomly chosen by the loader or the
/// boot compatibility layer. But we disabled it because the framework
/// doesn't support relocatable kernel yet.
pub const fn kernel_loaded_offset() -> usize {
    0xffff_ffff_8000_0000
}
const_assert!(LINEAR_MAPPING_VADDR_RANGE.end < kernel_loaded_offset());

/// Convert physical address to virtual address using offset, only available inside aster-frame
pub(crate) fn paddr_to_vaddr(pa: Paddr) -> usize {
    pa + LINEAR_MAPPING_BASE_VADDR
}

pub static KERNEL_PAGE_TABLE: Once<PageTable<KernelMode, PageTableEntry, PagingConsts>> =
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
        nr_ptes_per_node::<PagingConsts>() / 2..nr_ptes_per_node::<PagingConsts>(),
    );
    let regions = crate::boot::memory_regions();

    // Do linear mappings for the kernel.
    {
        let linear_mapping_size = {
            let mut end = 0;
            for r in regions {
                end = end.max(r.base() + r.len());
            }
            end.align_up(PAGE_SIZE)
        };
        let from = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + linear_mapping_size;
        let to = 0..linear_mapping_size;
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
        // SAFETY: we are doing mappings for the kernel.
        unsafe {
            kpt.map(&from, &to, prop).unwrap();
        }
    }

    KERNEL_PAGE_TABLE.call_once(|| kpt);
}
