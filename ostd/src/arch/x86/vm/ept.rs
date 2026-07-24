// SPDX-License-Identifier: MPL-2.0

#![allow(
    dead_code,
    reason = "EPT support is being wired in stages, so this page-table config is not called yet."
)]

use core::ops::Range;

use crate::mm::{
    AnyUFrameMeta, Frame, HasPaddr, Paddr, PageProperty, PagingConstsTrait, PagingLevel, PodOnce,
    UFrame,
    frame::FrameRef,
    page_prop::{CachePolicy, PageFlags, PageTableFlags, PrivilegedPageFlags as PrivFlags},
    page_table::{PageTableConfig, PteScalar, PteTrait},
};

/// The page table used by ept maps the guest physical address
/// space to the host physical address space.
#[derive(Clone, Debug)]
pub struct EptPtConfig {}

// SAFETY: `item_raw_info`, `item_into_raw`, `item_from_raw`, and
// `item_ref_from_raw` are correctly implemented with respect to the `Item` and
// `ItemRef` types.
unsafe impl PageTableConfig for EptPtConfig {
    // 1 for 512GB, 256 is enough.
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..256;

    type E = PageTableEntry;
    type C = PagingConsts;

    /// All mappings are tracked untyped frames.
    type Item = EptItem;
    type ItemRef<'a> = EptItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        let (frame, prop) = item;
        (frame.paddr(), frame.map_level(), *prop)
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        debug_assert_eq!(level, 1);
        // SAFETY: The caller ensures that the raw item was produced from a
        // `UFrame` previously consumed by this page table.
        let frame = unsafe { Frame::<dyn AnyUFrameMeta>::from_raw(paddr) };
        (frame, prop)
    }

    unsafe fn item_ref_from_raw<'a>(
        paddr: Paddr,
        level: PagingLevel,
        prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        debug_assert_eq!(level, 1);
        // SAFETY: The caller ensures that the mapped frame outlives `'a`.
        let frame = unsafe { FrameRef::<dyn AnyUFrameMeta>::borrow_paddr(paddr) };
        (frame, prop)
    }
}

pub(crate) type EptItem = (UFrame, PageProperty);
pub(crate) type EptItemRef<'a> = (FrameRef<'a, dyn AnyUFrameMeta>, PageProperty);

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 1;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct PteFlags: usize {
        /// Memory type: Write-Back (WB).
        const MEM_TYPE_WB =     6 << 3;

        /// Ignore PAT.
        const IGNORE_PAT =      1 << 6;

        const WRITABLE =        1 << 1;

        const READABLE =        1 << 0;

        const EXECUTABLE =      1 << 2;
    }
}

/// Parses a bit-flag bits `val` in the representation of `from` to `to` in bits.
macro_rules! parse_flags {
    ($val:expr, $from:expr, $to:expr) => {
        (($val as usize & $from.bits() as usize) >> $from.bits().ilog2() << $to.bits().ilog2())
    };
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct PageTableEntry(usize);

impl PageTableEntry {
    const PHYS_MASK: usize = 0xf_ffff_ffff_f000;

    fn is_present(&self) -> bool {
        self.0 & (PteFlags::READABLE | PteFlags::WRITABLE | PteFlags::EXECUTABLE).bits() != 0
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1
    }

    fn prop(&self) -> PageProperty {
        let flags = parse_flags!(self.0, PteFlags::READABLE, PageFlags::R)
            | parse_flags!(self.0, PteFlags::WRITABLE, PageFlags::W)
            | parse_flags!(self.0, PteFlags::EXECUTABLE, PageFlags::X);

        // TODO: The determination cache policy is not rigorous. We should revise it.
        let cache = if self.0 & PteFlags::MEM_TYPE_WB.bits() != 0 {
            CachePolicy::Writeback
        } else {
            CachePolicy::Uncacheable
        };

        PageProperty {
            flags: PageFlags::from_bits(flags as u8).unwrap(),
            cache,
            priv_flags: PrivFlags::empty(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        PageTableFlags::empty()
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::IGNORE_PAT.bits()
            | parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::READABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::W, PteFlags::WRITABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::X, PteFlags::EXECUTABLE);

        if prop.cache != CachePolicy::Uncacheable {
            flags |= PteFlags::MEM_TYPE_WB.bits();
        }

        Self(paddr & Self::PHYS_MASK | flags)
    }

    fn new_pt(paddr: Paddr, _flags: PageTableFlags) -> Self {
        // TODO: currently ignore the flags argument.
        let flags =
            PteFlags::READABLE.bits() | PteFlags::WRITABLE.bits() | PteFlags::EXECUTABLE.bits();
        Self(paddr & Self::PHYS_MASK | flags)
    }
}

impl PodOnce for PageTableEntry {}

// SAFETY: The implementation is safe because:
// - `from_usize` and `into_usize` are not overridden;
// - `from_repr` and `repr` are correctly implemented;
// - a zeroed PTE represents an absent entry.
unsafe impl PteTrait for PageTableEntry {
    fn from_repr(repr: &PteScalar, level: PagingLevel) -> Self {
        match repr {
            PteScalar::Absent => PageTableEntry(0),
            PteScalar::PageTable(paddr, flags) => Self::new_pt(*paddr, *flags),
            PteScalar::Mapped(paddr, prop) => Self::new_page(*paddr, level, *prop),
        }
    }

    fn to_repr(&self, level: PagingLevel) -> PteScalar {
        if !self.is_present() {
            return PteScalar::Absent;
        }

        let paddr = self.0 & Self::PHYS_MASK;
        if self.is_last(level) {
            PteScalar::Mapped(paddr, self.prop())
        } else {
            PteScalar::PageTable(paddr, self.pt_flags())
        }
    }
}
