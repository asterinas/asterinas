// SPDX-License-Identifier: MPL-2.0

//! Virtual memory (VM).

#![cfg_attr(
    any(
        target_arch = "riscv64",
        target_arch = "loongarch64",
        target_arch = "aarch64"
    ),
    expect(unused_imports)
)]

pub mod dma;
pub mod fault;
pub mod frame;
pub mod heap;
pub mod io;
pub(crate) mod kspace;
pub(crate) mod mem_obj;
pub(crate) mod page_prop;
pub(crate) mod page_table;
pub mod tlb;
pub mod vm_space;

#[cfg(ktest)]
mod test;

use core::fmt::Debug;

pub use self::{
    frame::{
        Frame,
        allocator::FrameAllocOptions,
        segment::{Segment, USegment},
        unique::UniqueFrame,
        untyped::{AnyUFrameMeta, UFrame},
    },
    io::{
        Fallible, FallibleVmRead, FallibleVmWrite, Infallible, PodAtomic, PodOnce, VmIo, VmIoFill,
        VmIoOnce, VmReader, VmWriter,
    },
    kspace::{KERNEL_VADDR_RANGE, MAX_USERSPACE_VADDR},
    mem_obj::{HasDaddr, HasPaddr, HasPaddrRange, HasSize, Split},
    page_prop::{CachePolicy, PageFlags, PageProperty},
    vm_space::VmSpace,
};
pub(crate) use self::{
    kspace::paddr_to_vaddr,
    page_prop::{PageTableFlags, PrivilegedPageFlags},
    page_table::PageTable,
};
use crate::arch::mm::PagingConsts;

/// Virtual addresses.
pub type Vaddr = usize;

/// Physical addresses.
pub type Paddr = usize;

/// Device addresses.
pub type Daddr = usize;

/// Returns whether `vaddr` and `len` specify a user-space virtual address range.
pub fn is_in_user_space(vaddr: Vaddr, len: usize) -> bool {
    vaddr.saturating_add(len) <= MAX_USERSPACE_VADDR
}

/// The level of a page table node or a frame.
pub type PagingLevel = u8;

/// A minimal set of constants that determines the paging system.
/// This provides an abstraction over most paging modes in common architectures.
pub(crate) trait PagingConstsTrait: Clone + Debug + Send + Sync + 'static {
    /// The smallest page size.
    /// This is also the page size at level 1 page tables.
    const BASE_PAGE_SIZE: usize;

    /// The VPN index width at each level, starting with level 1.
    ///
    /// Each node contains `1 << VPN_BITS[level - 1]` entries. For example,
    /// Sv39 uses `&[9, 9, 9]`, while Sv39x4 uses `&[9, 9, 11]`.
    const VPN_BITS: &'static [usize];

    /// The highest level that a PTE can be directly used to translate a VA.
    /// This affects the the largest page size supported by the page table.
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel;

    /// The size of a PTE.
    const PTE_SIZE: usize;

    /// The address width may be `BASE_PAGE_SIZE.ilog2() + VPN_BITS.iter().sum()`.
    /// If it is shorter than that, the higher bits in the highest level are ignored.
    const ADDRESS_WIDTH: usize;

    /// Whether virtual addresses are sign-extended.
    ///
    /// The sign bit of a [`Vaddr`] is the bit at index [`PagingConstsTrait::ADDRESS_WIDTH`] - 1.
    /// If this constant is `true`, bits in [`Vaddr`] that are higher than the sign bit must be
    /// equal to the sign bit. If an address violates this rule, both the hardware and OSTD
    /// should reject it.
    ///
    /// Otherwise, if this constant is `false`, higher bits must be zero.
    ///
    /// Regardless of sign extension, [`Vaddr`] is always not signed upon calculation.
    /// That means, `0xffff_ffff_ffff_0000 < 0xffff_ffff_ffff_0001` is `true`.
    const VA_SIGN_EXT: bool;
}

/// The page size
pub const PAGE_SIZE: usize = page_size::<PagingConsts>(1);

/// Returns the number of page table levels, derived from the VPN index widths.
///
/// Levels are numbered from the deepest node (level 1) to the root.
pub(crate) const fn nr_levels<C: PagingConstsTrait>() -> PagingLevel {
    let nr_levels = C::VPN_BITS.len();
    assert!(nr_levels > 0 && nr_levels <= PagingLevel::MAX as usize);
    nr_levels as PagingLevel
}

/// The page size at a given level.
pub(crate) const fn page_size<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    let mut size = C::BASE_PAGE_SIZE;
    let mut lower_level = 1;
    while lower_level < level {
        size <<= C::VPN_BITS[lower_level as usize - 1];
        lower_level += 1;
    }
    size
}

/// Returns the number of entries in a page table node at the given level.
pub(crate) const fn nr_ptes<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    let bits = C::VPN_BITS[level as usize - 1];
    assert!(bits < usize::BITS as usize);
    1 << bits
}

/// Returns the size and alignment of a page table node at the given level.
pub(crate) const fn page_table_node_size<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    let size = nr_ptes::<C>(level)
        .checked_mul(C::PTE_SIZE)
        .expect("page table node size overflows");
    if size < PAGE_SIZE { PAGE_SIZE } else { size }
}

/// The number of base pages in a huge page at a given level.
#[expect(dead_code)]
pub(crate) const fn nr_base_per_page<C: PagingConstsTrait>(level: PagingLevel) -> usize {
    page_size::<C>(level) / C::BASE_PAGE_SIZE
}
