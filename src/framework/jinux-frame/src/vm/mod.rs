//! Virtual memory (VM).

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

mod frame;
mod frame_allocator;
mod heap_allocator;
mod io;
mod memory_set;
mod offset;
pub(crate) mod page_table;
mod space;

use crate::config::{PAGE_SIZE, PHYS_OFFSET};

pub use self::frame::{VmAllocOptions, VmFrame, VmFrameVec, VmFrameVecIter};
pub use self::io::VmIo;
pub use self::space::{VmMapOptions, VmPerm, VmSpace};

pub use self::{
    memory_set::{MapArea, MemorySet},
    page_table::PageTable,
};

use alloc::vec::Vec;
use spin::Once;

#[derive(Clone, Copy)]
pub struct MemoryRegions {
    pub base: u64,
    pub len: u64,
    pub typ: MemoryRegionsType,
}

#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
/// Copy from limine boot protocol
pub enum MemoryRegionsType {
    Usable = 0,
    Reserved = 1,
    AcpiReclaimable = 2,
    AcpiNvs = 3,
    BadMemory = 4,
    BootloaderReclaimable = 5,
    /// The kernel and modules loaded are not marked as usable memory. They are
    /// marked as Kernel/Modules. The entries are guaranteed to be sorted by base
    /// address, lowest to highest. Usable and bootloader reclaimable entries are
    /// guaranteed to be 4096 byte aligned for both base and length. Usable and
    /// bootloader reclaimable entries are guaranteed not to overlap with any
    /// other entry. To the contrary, all non-usable entries (including kernel/modules)
    /// are not guaranteed any alignment, nor is it guaranteed that they do not
    /// overlap other entries.
    KernelAndModules = 6,
    Framebuffer = 7,
}

/// Convert physical address to virtual address using offset, only available inside jinux-frame
pub(crate) fn paddr_to_vaddr(pa: usize) -> usize {
    pa + PHYS_OFFSET
}

pub fn vaddr_to_paddr(va: Vaddr) -> Option<Paddr> {
    if va >= crate::config::PHYS_OFFSET && va <= crate::config::KERNEL_OFFSET {
        // can use offset to get the physical address
        Some(va - PHYS_OFFSET)
    } else {
        page_table::vaddr_to_paddr(va)
    }
}

pub const fn is_page_aligned(p: usize) -> bool {
    (p & (PAGE_SIZE - 1)) == 0
}

/// Only available inside jinux-frame
pub(crate) static MEMORY_REGIONS: Once<&Vec<MemoryRegions>> = Once::new();

pub static FRAMEBUFFER_REGIONS: Once<Vec<MemoryRegions>> = Once::new();

pub(crate) fn init() {
    heap_allocator::init();
    #[cfg(feature = "x86_64")]
    let memory_regions = crate::arch::x86::mm::get_memory_regions();

    let mut framebuffer_regions = Vec::new();
    for i in memory_regions.iter() {
        if i.typ == MemoryRegionsType::Framebuffer {
            framebuffer_regions.push(i.clone());
        }
    }

    frame_allocator::init(memory_regions);
    page_table::init();

    MEMORY_REGIONS.call_once(|| memory_regions);
    FRAMEBUFFER_REGIONS.call_once(|| framebuffer_regions);
}
