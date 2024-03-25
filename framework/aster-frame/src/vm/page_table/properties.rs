// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use pod::Pod;

use crate::vm::{Paddr, Vaddr, VmPerm};

/// A minimal set of constants that determines the flags of the page table.
/// This provides an abstraction over most paging modes in common architectures.
pub trait PageTableConstsTrait: Debug + 'static {
    /// The smallest page size.
    const BASE_PAGE_SIZE: usize;

    /// The number of levels in the page table.
    /// The level 1 is the leaf level, and the level `NR_LEVELS` is the root level.
    const NR_LEVELS: usize;

    /// The highest level that a PTE can be directly used to translate a VA.
    /// This affects the the largest page size supported by the page table.
    const HIGHEST_TRANSLATION_LEVEL: usize;

    /// The size of a PTE.
    const ENTRY_SIZE: usize;

    // Here are some const values that are determined by the page table constants.

    /// The number of PTEs per page table frame.
    const NR_ENTRIES_PER_FRAME: usize = Self::BASE_PAGE_SIZE / Self::ENTRY_SIZE;

    /// The number of bits used to index a PTE in a page table frame.
    const IN_FRAME_INDEX_BITS: usize = Self::NR_ENTRIES_PER_FRAME.ilog2() as usize;

    /// The index of a VA's PTE in a page table frame at the given level.
    fn in_frame_index(va: Vaddr, level: usize) -> usize {
        va >> (Self::BASE_PAGE_SIZE.ilog2() as usize + Self::IN_FRAME_INDEX_BITS * (level - 1))
            & (Self::NR_ENTRIES_PER_FRAME - 1)
    }

    /// The page size at a given level.
    fn page_size(level: usize) -> usize {
        Self::BASE_PAGE_SIZE << (Self::IN_FRAME_INDEX_BITS * (level - 1))
    }
}

bitflags::bitflags! {
    /// The status of a memory mapping recorded by the hardware.
    pub struct MapStatus: u32 {
        const ACCESSED = 0b0000_0001;
        const DIRTY    = 0b0000_0010;
    }
}

// TODO: Make it more abstract when supporting other architectures.
/// A type to control the cacheability of the main memory.
///
/// The type currently follows the definition as defined by the AMD64 manual.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum CachePolicy {
    /// Uncacheable (UC).
    ///
    /// Reads from, and writes to, UC memory are not cacheable.
    /// Reads from UC memory cannot be speculative.
    /// Write-combining to UC memory is not allowed.
    /// Reads from or writes to UC memory cause the write buffers to be written to memory
    /// and be invalidated prior to the access to UC memory.
    ///
    /// The UC memory type is useful for memory-mapped I/O devices
    /// where strict ordering of reads and writes is important.
    Uncacheable,
    /// Write-Combining (WC).
    ///
    /// Reads from, and writes to, WC memory are not cacheable.
    /// Reads from WC memory can be speculative.
    ///
    /// Writes to this memory type can be combined internally by the processor
    /// and written to memory as a single write operation to reduce memory accesses.
    ///
    /// The WC memory type is useful for graphics-display memory buffers
    /// where the order of writes is not important.
    WriteCombining,
    /// Write-Protect (WP).
    ///
    /// Reads from WP memory are cacheable and allocate cache lines on a read miss.
    /// Reads from WP memory can be speculative.
    ///
    /// Writes to WP memory that hit in the cache do not update the cache.
    /// Instead, all writes update memory (write to memory),
    /// and writes that hit in the cache invalidate the cache line.
    /// Write buffering of WP memory is allowed.
    ///
    /// The WP memory type is useful for shadowed-ROM memory
    /// where updates must be immediately visible to all devices that read the shadow locations.
    WriteProtected,
    /// Writethrough (WT).
    ///
    /// Reads from WT memory are cacheable and allocate cache lines on a read miss.
    /// Reads from WT memory can be speculative.
    ///
    /// All writes to WT memory update main memory,
    /// and writes that hit in the cache update the cache line.
    /// Writes that miss the cache do not allocate a cache line.
    /// Write buffering of WT memory is allowed.
    Writethrough,
    /// Writeback (WB).
    ///
    /// The WB memory is the "normal" memory. See detailed descriptions in the manual.
    ///
    /// This type of memory provides the highest-possible performance
    /// and is useful for most software and data stored in system memory (DRAM).
    Writeback,
}

#[derive(Clone, Copy, Debug)]
pub struct MapProperty {
    pub perm: VmPerm,
    pub cache: CachePolicy,
}

/// Any functions that could be used to modify the map property of a memory mapping.
pub trait MapOp: Fn(MapInfo) -> MapProperty {}
impl<F> MapOp for F where F: Fn(MapInfo) -> MapProperty {}

// These implementations allow a property to be used as an overriding map operation.
// Other usages seems pointless.
impl FnOnce<(MapInfo,)> for MapProperty {
    type Output = MapProperty;
    extern "rust-call" fn call_once(self, _: (MapInfo,)) -> MapProperty {
        self
    }
}
impl FnMut<(MapInfo,)> for MapProperty {
    extern "rust-call" fn call_mut(&mut self, _: (MapInfo,)) -> MapProperty {
        *self
    }
}
impl Fn<(MapInfo,)> for MapProperty {
    extern "rust-call" fn call(&self, _: (MapInfo,)) -> MapProperty {
        *self
    }
}

impl MapProperty {
    pub fn new_invalid() -> Self {
        Self {
            perm: VmPerm::empty(),
            cache: CachePolicy::Uncacheable,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct MapInfo {
    pub prop: MapProperty,
    pub status: MapStatus,
}

pub trait PageTableEntryTrait: Clone + Copy + Sized + Pod + Debug {
    /// Create a new invalid page table flags that causes page faults
    /// when the MMU meets them.
    fn new_invalid() -> Self;
    /// If the flags are valid.
    /// Note that the invalid PTE may be _valid_ in representation, but
    /// just causing page faults when the MMU meets them.
    fn is_valid(&self) -> bool;

    /// Create a new PTE with the given physical address and flags.
    /// The huge flag indicates that the PTE maps a huge page.
    /// The last flag indicates that the PTE is the last level page table.
    /// If the huge and last flags are both false, the PTE maps a page
    /// table frame.
    fn new(paddr: Paddr, prop: MapProperty, huge: bool, last: bool) -> Self;

    /// Get the physical address from the PTE.
    /// The physical address recorded in the PTE is either:
    /// - the physical address of the next level page table;
    /// - or the physical address of the page frame it maps to.
    fn paddr(&self) -> Paddr;

    fn info(&self) -> MapInfo;

    /// If the PTE maps a huge page or a page table frame.
    fn is_huge(&self) -> bool;
}
