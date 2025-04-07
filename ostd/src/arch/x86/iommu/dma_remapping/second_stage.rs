// SPDX-License-Identifier: MPL-2.0

#![expect(unused_variables)]

use core::ops::Range;

use crate::{
    mm::{
        page_prop::{CachePolicy, PageFlags, PrivilegedPageFlags as PrivFlags},
        page_table::{PageTableConfig, PageTableEntryTrait},
        Paddr, PageProperty, PagingConstsTrait, PagingLevel, PodOnce,
    },
    Pod,
};

/// The page table used by iommu maps the device address
/// space to the physical address space.
#[derive(Clone, Debug)]
pub(crate) struct IommuPtConfig {}

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
    const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    pub struct PageTableFlags : u64{
        /// Whether accesses to this page must snoop processor caches.
        const SNOOP =           1 << 11;

        const DIRTY =           1 << 9;

        const ACCESSED =        1 << 8;
        /// Whether this page table entry is the last entry.
        const LAST_PAGE =       1 << 7;

        /// Ignore PAT, 1 if the scalable-mode PASID-table entry is not
        /// used for effective memory-type determination.
        const IGNORE_PAT =     1 << 6;

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
pub struct PageTableEntry(u64);

impl PageTableEntry {
    const PHYS_MASK: u64 = 0xFFFF_FFFF_F000;
    const PROP_MASK: u64 = !Self::PHYS_MASK & !PageTableFlags::LAST_PAGE.bits();
}

impl PodOnce for PageTableEntry {}

impl PageTableEntryTrait for PageTableEntry {
    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        let flags = if prop.has_map {
            PageTableFlags::LAST_PAGE.bits()
        } else {
            PageTableFlags::empty().bits()
        };
        let mut pte = Self(paddr as u64 & Self::PHYS_MASK | flags);
        pte.set_prop(prop);
        pte
    }

    fn new_pt(paddr: Paddr) -> Self {
        Self(
            paddr as u64 & Self::PHYS_MASK
                | PageTableFlags::READABLE.bits()
                | PageTableFlags::WRITABLE.bits(),
        )
    }

    fn paddr(&self) -> Paddr {
        (self.0 & Self::PHYS_MASK) as usize
    }

    fn is_present(&self) -> bool {
        self.0 & (PageTableFlags::READABLE | PageTableFlags::WRITABLE).bits() != 0
    }

    fn prop(&self) -> PageProperty {
        let mut flags = PageFlags::empty();
        if self.0 & PageTableFlags::READABLE.bits() != 0 {
            flags |= PageFlags::R;
        }
        if self.0 & PageTableFlags::WRITABLE.bits() != 0 {
            flags |= PageFlags::W;
        }
        if self.0 & PageTableFlags::ACCESSED.bits() != 0 {
            flags |= PageFlags::ACCESSED;
        }
        if self.0 & PageTableFlags::DIRTY.bits() != 0 {
            flags |= PageFlags::DIRTY;
        }
        // TODO: The determination cache policy is not rigorous. We should revise it.
        let cache = if self.0 & PageTableFlags::SNOOP.bits() != 0 {
            CachePolicy::Writeback
        } else {
            CachePolicy::Uncacheable
        };

        PageProperty {
            has_map: true,
            flags,
            cache,
            priv_flags: PrivFlags::empty(),
        }
    }

    fn set_prop(&mut self, prop: PageProperty) {
        let mut flags = PageTableFlags::empty();
        if prop.flags.contains(PageFlags::W) {
            flags |= PageTableFlags::WRITABLE;
        }
        if prop.flags.contains(PageFlags::R) {
            flags |= PageTableFlags::READABLE;
        }
        if prop.cache != CachePolicy::Uncacheable {
            flags |= PageTableFlags::SNOOP;
        }
        self.0 = self.0 & !Self::PROP_MASK | flags.bits();
    }

    fn set_paddr(&mut self, paddr: Paddr) {
        self.0 = self.0 & !Self::PHYS_MASK | (paddr as u64 & Self::PHYS_MASK);
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1
    }
}
