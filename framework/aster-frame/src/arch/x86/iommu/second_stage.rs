// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use pod::Pod;

use crate::vm::{
    page_table::{
        CachePolicy, MapInfo, MapProperty, MapStatus, PageTableConstsTrait, PageTableEntryTrait,
        PageTableMode,
    },
    Paddr, Vaddr, VmPerm,
};

/// The page table used by iommu maps the device address
/// space to the physical address space.
#[derive(Clone)]
pub(super) struct DeviceMode {}

impl PageTableMode for DeviceMode {
    /// The device address space is 32-bit.
    const VADDR_RANGE: Range<Vaddr> = 0..0x1_0000_0000;
}

#[derive(Debug)]
pub(super) struct PageTableConsts {}

impl PageTableConstsTrait for PageTableConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: usize = 3;
    const HIGHEST_TRANSLATION_LEVEL: usize = 1;
    const ENTRY_SIZE: usize = core::mem::size_of::<PageTableEntry>();
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

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct PageTableEntry(u64);

impl PageTableEntry {
    const PHYS_MASK: usize = 0xFFFF_FFFF_F000;
}

impl PageTableEntryTrait for PageTableEntry {
    fn new(paddr: crate::vm::Paddr, prop: MapProperty, huge: bool, last: bool) -> Self {
        let mut flags = PageTableFlags::empty();
        if prop.perm.contains(VmPerm::W) {
            flags |= PageTableFlags::WRITABLE;
        }
        if prop.perm.contains(VmPerm::R) {
            flags |= PageTableFlags::READABLE;
        }
        if last {
            flags |= PageTableFlags::LAST_PAGE;
        }
        if huge {
            panic!("Huge page is not supported in iommu page table");
        }
        Self((paddr & Self::PHYS_MASK) as u64 | flags.bits)
    }

    fn paddr(&self) -> Paddr {
        (self.0 & Self::PHYS_MASK as u64) as usize
    }

    fn new_invalid() -> Self {
        Self(0)
    }

    fn is_valid(&self) -> bool {
        self.0 & (PageTableFlags::READABLE | PageTableFlags::WRITABLE).bits() != 0
    }

    fn info(&self) -> MapInfo {
        let mut perm = VmPerm::empty();
        if self.0 & PageTableFlags::READABLE.bits() != 0 {
            perm |= VmPerm::R;
        }
        if self.0 & PageTableFlags::WRITABLE.bits() != 0 {
            perm |= VmPerm::W;
        }
        let cache = if self.0 & PageTableFlags::SNOOP.bits() != 0 {
            CachePolicy::Writeback
        } else {
            CachePolicy::Uncacheable
        };
        let mut status = MapStatus::empty();
        if self.0 & PageTableFlags::ACCESSED.bits() != 0 {
            status |= MapStatus::ACCESSED;
        }
        if self.0 & PageTableFlags::DIRTY.bits() != 0 {
            status |= MapStatus::DIRTY;
        }
        MapInfo {
            prop: MapProperty { perm, cache },
            status,
        }
    }

    fn is_huge(&self) -> bool {
        false
    }
}
