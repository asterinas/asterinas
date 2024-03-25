// SPDX-License-Identifier: MPL-2.0

use core::fmt::Debug;

use pod::Pod;

use crate::vm::{Paddr, Vaddr};

/// A minimal set of constants that determines the flags of the page table.
/// This provides an abstraction over most paging modes in common architectures.
pub trait PageTableConstsTrait {
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
}

// Here are some const values that are determined by the page table constants.

/// The number of PTEs per page table frame.
pub(super) const fn nr_entries_per_frame<P: PageTableConstsTrait>() -> usize {
    P::BASE_PAGE_SIZE / P::ENTRY_SIZE
}

/// The number of bits used to index a PTE in a page table frame.
pub(super) const fn in_frame_index_bits<P: PageTableConstsTrait>() -> usize {
    nr_entries_per_frame::<P>().ilog2()
}

/// The index of a VA's PTE in a page table frame at the given level.
pub(super) const fn in_frame_index<P: PageTableConstsTrait>(va: Vaddr, level: usize) -> usize {
    va >> (P::BASE_PAGE_SIZE.ilog2() + in_frame_index_bits::<P>() * (level - 1))
        & (P::nr_entries_per_frame() - 1)
}

/// The page size at a given level.
pub(super) const fn page_size<P: PageTableConstsTrait>(level: usize) -> usize {
    P::BASE_PAGE_SIZE << (in_frame_index_bits::<P>() * (level - 1))
}

bitflags::bitflags! {
    /// The flags of a memory mapping.
    #[derive(Clone, Copy, Debug)]
    pub struct MapFlags: u32 {
        const READ      = 0b0000_0001;
        const WRITE     = 0b0000_0010;
        const EXEC      = 0b0000_0100;
        const USER      = 0b0000_1000;
        const GLOBAL    = 0b0001_0000;
    }
}

bitflags::bitflags! {
    /// The status of a memory mapping recorded by the hardware.
    #[derive(Clone, Copy, Debug)]
    pub struct MapStatus: u32 {
        const ACCESSED = 0b0000_0001;
        const DIRTY    = 0b0000_0010;
    }
}

/// The cache policy of a memory mapping.
/// FIXME: This may not be supported by all architectures and could be
/// ignored by us without warnings at the moment.
#[derive(Clone, Copy, Debug)]
pub enum MapCachePolicy {
    Uncachable,
    WriteCombining,
    WriteThrough,
    WriteBack,
    WriteProtected,
}

#[derive(Clone, Copy, Debug)]
pub struct MapProperty {
    pub flags: MapFlags,
    pub cache: MapCachePolicy,
}

impl MapProperty {
    pub fn new_invalid() -> Self {
        Self {
            flags: MapFlags::empty(),
            cache: MapCachePolicy::Uncachable,
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
