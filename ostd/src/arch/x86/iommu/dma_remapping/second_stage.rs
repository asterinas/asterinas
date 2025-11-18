// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use crate::{
    Pod,
    mm::{
        Paddr, PageProperty, PagingConstsTrait, PagingLevel, PodOnce,
        page_prop::{CachePolicy, PageFlags, PageTableFlags, PrivilegedPageFlags as PrivFlags},
        page_table::{PageTableConfig, PteScalar, PteTrait},
    },
};

/// The page table used by iommu maps the device address
/// space to the physical address space.
#[derive(Clone, Debug)]
pub struct IommuPtConfig {}

// SAFETY: `item_into_raw` and `item_from_raw` are implemented correctly,
unsafe impl PageTableConfig for IommuPtConfig {
    /// From section 3.6 in "Intel(R) Virtualization Technology for Directed I/O",
    /// only low canonical addresses can be used.
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..256;

    type E = PageTableEntry;
    type C = PagingConsts;

    /// All mappings are untracked.
    type Item = (Paddr, PagingLevel, PageProperty);

    fn item_into_raw(item: Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        item
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        (paddr, level, prop)
    }
}

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 3;
    const ADDRESS_WIDTH: usize = 39;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 1;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    pub struct PteFlags: usize {
        /// Whether accesses to this page must snoop processor caches.
        const SNOOP =           1 << 11;

        /// Bits ignored by hardware.
        const IGN2 =            1 << 9;
        const IGN1 =            1 << 8;

        /// Whether this page table entry is the last entry.
        const LAST_PAGE =       1 << 7;

        /// Ignore PAT, 1 if the scalable-mode PASID-table entry is not
        /// used for effective memory-type determination.
        const IGNORE_PAT =      1 << 6;

        /// Extended Memory Type, ignored by hardware when the
        /// Extended Memory Type Enable (EMTE) field is Clear.
        ///
        /// When the EMTE field is Set, this field is used to compute effective
        /// memory-type for second-stage-only and nested translations.
        const EMT =             7 << 3;

        const WRITABLE =        1 << 1;

        const READABLE =        1 << 0;

    }
}

#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct PageTableEntry(usize);

/// Parses a bit-flag bits `val` in the representation of `from` to `to` in bits.
macro_rules! parse_flags {
    ($val:expr, $from:expr, $to:expr) => {
        (($val as usize & $from.bits() as usize) >> $from.bits().ilog2() << $to.bits().ilog2())
    };
}

impl PageTableEntry {
    const PHYS_MASK: usize = 0xffff_ffff_f000;

    fn is_present(&self) -> bool {
        self.0 & (PteFlags::READABLE | PteFlags::LAST_PAGE).bits() != 0
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1
    }

    fn prop(&self) -> PageProperty {
        let flags = parse_flags!(self.0, PteFlags::READABLE, PageFlags::R)
            | parse_flags!(self.0, PteFlags::WRITABLE, PageFlags::W)
            | parse_flags!(self.0, PteFlags::IGN2, PageFlags::AVAIL2);

        let priv_flags = parse_flags!(self.0, PteFlags::IGN1, PrivFlags::AVAIL1);

        // TODO: The determination cache policy is not rigorous. We should revise it.
        let cache = if self.0 & PteFlags::SNOOP.bits() != 0 {
            CachePolicy::Writeback
        } else {
            CachePolicy::Uncacheable
        };

        PageProperty {
            flags: PageFlags::from_bits(flags as u8).unwrap(),
            cache,
            priv_flags: PrivFlags::from_bits(priv_flags as u8).unwrap(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        let bits = PageTableFlags::empty().bits() as usize
            | parse_flags!(self.0, PteFlags::IGN1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::IGN2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::LAST_PAGE.bits()
            | parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::READABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::W, PteFlags::WRITABLE)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::AVAIL1, PteFlags::IGN1)
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::IGN2);

        if prop.cache != CachePolicy::Uncacheable {
            flags |= PteFlags::SNOOP.bits();
        }

        Self(paddr & Self::PHYS_MASK | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let flags = PteFlags::READABLE.bits()
            | PteFlags::WRITABLE.bits()
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL1, PteFlags::IGN1)
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL2, PteFlags::IGN2);
        Self(paddr & Self::PHYS_MASK | flags)
    }
}

impl PodOnce for PageTableEntry {}

/// SAFETY: The implementation is safe because:
///  - `from_usize` and `into_usize` are not overridden;
///  - `from_repr` and `repr` are correctly implemented;
///  - a zeroed PTE represents an absent entry.
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
