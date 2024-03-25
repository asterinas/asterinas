// SPDX-License-Identifier: MPL-2.0

//! Virtual memory (VM).

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

pub(crate) mod dma;
mod frame;
mod frame_allocator;
pub(crate) mod heap_allocator;
mod io;
pub(crate) mod kspace;
mod memory_set;
mod offset;
mod options;
pub(crate) mod page_table;
mod space;

use alloc::{borrow::ToOwned, vec::Vec};
use core::ops::Range;

use spin::Once;

pub(crate) use self::kspace::paddr_to_vaddr;
pub use self::{
    dma::{Daddr, DmaCoherent, DmaDirection, DmaStream, DmaStreamSlice, HasDaddr},
    frame::{VmFrame, VmFrameVec, VmFrameVecIter, VmReader, VmSegment, VmWriter},
    io::VmIo,
    kspace::vaddr_to_paddr,
    memory_set::{MapArea, MemorySet},
    options::VmAllocOptions,
    page_table::PageTable,
    space::{VmMapOptions, VmPerm, VmSpace},
};
use crate::boot::memory_region::{MemoryRegion, MemoryRegionType};

pub const PAGE_SIZE: usize = 0x1000;

/// The maximum virtual address of user space (non inclusive).
///
/// Typicall 64-bit systems have at least 48-bit virtual address space.
/// A typical way to reserve half of the address space for the kernel is
/// to use the highest 48-bit virtual address space.
///
/// Also, the top page is not regarded as usable since it's a workaround
/// for some x86_64 CPUs' bugs. See
/// <https://github.com/torvalds/linux/blob/480e035fc4c714fb5536e64ab9db04fedc89e910/arch/x86/include/asm/page_64.h#L68-L78>
/// for the rationale.
pub const MAX_USERSPACE_VADDR: Vaddr = 0x0000_8000_0000_0000 - PAGE_SIZE;

/// The base address of the direct mapping of physical memory.
pub(crate) const PHYS_MEM_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000;

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
pub(crate) const PHYS_MEM_MAPPING_MAX_SIZE: usize = 127 << 40;

/// The address range of the direct mapping of physical memory.
///
/// This range is constructed based on [`PHYS_MEM_BASE_VADDR`] and
/// [`PHYS_MEM_MAPPING_MAX_SIZE`].
pub(crate) const PHYS_MEM_VADDR_RANGE: Range<Vaddr> =
    PHYS_MEM_BASE_VADDR..(PHYS_MEM_BASE_VADDR + PHYS_MEM_MAPPING_MAX_SIZE);

/// The kernel code is linear mapped to this address.
///
/// FIXME: This offset should be randomly chosen by the loader or the
/// boot compatibility layer. But we disabled it because the framework
/// doesn't support relocatable kernel yet.
pub const fn kernel_loaded_offset() -> usize {
    0xffff_ffff_8000_0000
}
const_assert!(PHYS_MEM_VADDR_RANGE.end < kernel_loaded_offset());

/// Start of the kernel address space.
/// This is the _lowest_ address of the x86-64's _high_ canonical addresses.
pub(crate) const KERNEL_BASE_VADDR: Vaddr = 0xffff_8000_0000_0000;
/// End of the kernel address space (non inclusive).
pub(crate) const KERNEL_END_VADDR: Vaddr = 0xffff_ffff_ffff_0000;

/// Get physical address trait
pub trait HasPaddr {
    fn paddr(&self) -> Paddr;
}

pub const fn is_page_aligned(p: usize) -> bool {
    (p & (PAGE_SIZE - 1)) == 0
}

/// Only available inside aster-frame
pub(crate) static MEMORY_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub static FRAMEBUFFER_REGIONS: Once<Vec<MemoryRegion>> = Once::new();

pub(crate) fn init() {
    let memory_regions = crate::boot::memory_regions().to_owned();
    frame_allocator::init(&memory_regions);
    kspace::init_kernel_page_table();
    dma::init();

    let mut framebuffer_regions = Vec::new();
    for i in memory_regions.iter() {
        if i.typ() == MemoryRegionType::Framebuffer {
            framebuffer_regions.push(*i);
        }
    }
    FRAMEBUFFER_REGIONS.call_once(|| framebuffer_regions);

    MEMORY_REGIONS.call_once(|| memory_regions);
}
