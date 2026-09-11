// SPDX-License-Identifier: MPL-2.0

//! Four-level Intel extended page tables with tracked 4-KiB backing frames.

use core::ops::Range;

use crate::mm::{
    AnyUFrameMeta, HasPaddr, Paddr, PageProperty, PagingConstsTrait, PagingLevel, PodOnce, UFrame,
    frame::{FrameRef, uframe_from_raw, uframe_ref_from_raw},
    page_prop::{CachePolicy, PageFlags, PageTableFlags, PrivilegedPageFlags as PrivFlags},
    page_table::{PageTableConfig, PteScalar, PteTrait},
};

/// A page table mapping guest physical addresses to host physical frames.
#[derive(Clone, Debug)]
pub(crate) struct EptPtConfig {}

unsafe impl PageTableConfig for EptPtConfig {
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..512;

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
        let frame = unsafe { uframe_from_raw(paddr) };
        (frame, prop)
    }

    unsafe fn item_ref_from_raw<'a>(
        paddr: Paddr,
        level: PagingLevel,
        prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        debug_assert_eq!(level, 1);
        // SAFETY: The caller ensures that the mapped frame outlives `'a`.
        let frame = unsafe { uframe_ref_from_raw(paddr) };
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
    const VA_SIGN_EXT: bool = false;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 1;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    struct PteFlags: usize {
        /// Ignore PAT.
        const IGNORE_PAT =      1 << 6;

        const WRITABLE =        1 << 1;

        const READABLE =        1 << 0;

        const EXECUTABLE =      1 << 2;

        const ACCESSED =        1 << 8;
        const DIRTY =           1 << 9;

        // Bits 11 and 56:52 are ignored by EPT hardware (Intel SDM, Vol. 3C,
        // Section 29.3.2). Preserve OSTD metadata without granting access.
        const PRESENT =         1 << 11;
        const AVAIL2 =          1 << 52;
        const USER =            1 << 53;
        const GLOBAL =          1 << 54;
        const AVAIL1 =          1 << 55;
        #[cfg(feature = "cvm_guest")]
        const SHARED =          1 << 56;
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
pub(crate) struct PageTableEntry(usize);

impl PageTableEntry {
    const PHYS_MASK: usize = 0xf_ffff_ffff_f000;

    fn is_present(&self) -> bool {
        self.0 & PteFlags::PRESENT.bits() != 0
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1
    }

    fn prop(&self) -> PageProperty {
        let flags = parse_flags!(self.0, PteFlags::READABLE, PageFlags::R)
            | parse_flags!(self.0, PteFlags::WRITABLE, PageFlags::W)
            | parse_flags!(self.0, PteFlags::EXECUTABLE, PageFlags::X)
            | parse_flags!(self.0, PteFlags::ACCESSED, PageFlags::ACCESSED)
            | parse_flags!(self.0, PteFlags::DIRTY, PageFlags::DIRTY)
            | parse_flags!(self.0, PteFlags::AVAIL2, PageFlags::AVAIL2);
        let priv_flags = parse_flags!(self.0, PteFlags::USER, PrivFlags::USER)
            | parse_flags!(self.0, PteFlags::GLOBAL, PrivFlags::GLOBAL)
            | parse_flags!(self.0, PteFlags::AVAIL1, PrivFlags::AVAIL1);
        #[cfg(feature = "cvm_guest")]
        let priv_flags = priv_flags | parse_flags!(self.0, PteFlags::SHARED, PrivFlags::SHARED);

        let cache = match (self.0 >> 3) & 0b111 {
            0 => CachePolicy::Uncacheable,
            1 => CachePolicy::WriteCombining,
            4 => CachePolicy::Writethrough,
            5 => CachePolicy::WriteProtected,
            6 => CachePolicy::Writeback,
            _ => unreachable!("invalid EPT memory type"),
        };

        PageProperty {
            flags: PageFlags::from_bits(flags as u8).unwrap(),
            cache,
            priv_flags: PrivFlags::from_bits(priv_flags as u8).unwrap(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        let flags = parse_flags!(self.0, PteFlags::AVAIL1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::AVAIL2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(flags as u8).unwrap()
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let flags = (PteFlags::PRESENT | PteFlags::IGNORE_PAT).bits()
            | parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::READABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::W, PteFlags::WRITABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::X, PteFlags::EXECUTABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::ACCESSED, PteFlags::ACCESSED)
            | parse_flags!(prop.flags.bits(), PageFlags::DIRTY, PteFlags::DIRTY)
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::AVAIL2)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::USER, PteFlags::USER)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::GLOBAL, PteFlags::GLOBAL)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::AVAIL1, PteFlags::AVAIL1);
        #[cfg(feature = "cvm_guest")]
        let flags =
            flags | parse_flags!(prop.priv_flags.bits(), PrivFlags::SHARED, PteFlags::SHARED);

        let memory_type = match prop.cache {
            CachePolicy::Uncacheable => 0,
            CachePolicy::WriteCombining => 1,
            CachePolicy::Writethrough => 4,
            CachePolicy::WriteProtected => 5,
            CachePolicy::Writeback => 6,
        };
        let flags = flags | (memory_type << 3);

        Self(paddr & Self::PHYS_MASK | flags)
    }

    fn new_pt(paddr: Paddr, pt_flags: PageTableFlags) -> Self {
        let flags =
            (PteFlags::PRESENT | PteFlags::READABLE | PteFlags::WRITABLE | PteFlags::EXECUTABLE)
                .bits()
                | parse_flags!(pt_flags.bits(), PageTableFlags::AVAIL1, PteFlags::AVAIL1)
                | parse_flags!(pt_flags.bits(), PageTableFlags::AVAIL2, PteFlags::AVAIL2);
        Self(paddr & Self::PHYS_MASK | flags)
    }
}

impl PodOnce for PageTableEntry {}

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

#[cfg(ktest)]
mod test {
    use super::*;
    use crate::prelude::*;

    #[ktest]
    fn leaf_encoding() {
        let paddr = 0x1234_5000;
        for (flags, cache, hardware_bits) in [
            (PageFlags::RW, CachePolicy::Writeback, 0b011 | (6 << 3)),
            (PageFlags::RX, CachePolicy::Uncacheable, 0b101),
        ] {
            let prop = PageProperty::new_user(flags, cache);
            let entry = PageTableEntry::from_repr(&PteScalar::Mapped(paddr, prop), 1);
            assert_eq!(entry.as_usize() & 0x3f, hardware_bits);
            assert_eq!(entry.as_usize() & PageTableEntry::PHYS_MASK, paddr);
            assert_eq!(entry.to_repr(1), PteScalar::Mapped(paddr, prop));
        }
    }
}
