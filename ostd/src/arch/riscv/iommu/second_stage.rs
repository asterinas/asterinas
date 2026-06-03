// SPDX-License-Identifier: MPL-2.0

//! Second-stage page table configuration for the RISC-V IOMMU (Sv39x4).
//!
//! PTE format follows the RISC-V privileged specification and matches the
//! kernel MMU's `PageTableEntry` layout in `ostd/src/arch/riscv/mm/mod.rs`.
//! The U (bit 4) and G (bit 5) flags are intentionally omitted: the IOMMU
//! treats U as "not requesting supervisor privilege," and G has no second-stage
//! semantic per the IOMMU spec.
//!
//! ## Root table limitation
//!
//! Full Sv39x4 per the spec requires a 16 KiB root table (2048 entries, 11-bit
//! VPN[2]) producing a 41-bit IOVA space. This implementation uses a 4 KiB root
//! table (512 entries, 9-bit VPN[2]), giving an effective 39-bit IOVA space.
//!
//! The smaller root table matches the kernel `PageTable` infrastructure's
//! assumption of uniform 4 KiB tables per level. For most virt/embedded use
//! cases (IOVA < 512 GiB) this is sufficient; the IOMMU only accesses the first
//! 4 KiB of the logical 16 KiB root region.

use core::{marker::PhantomData, ops::Range};

use crate::mm::{
    Paddr, PageProperty, PagingConstsTrait, PagingLevel, PodOnce,
    page_prop::{CachePolicy, PageFlags, PageTableFlags, PrivilegedPageFlags as PrivFlags},
    page_table::{PageTableConfig, PteScalar, PteTrait},
};

/// The IOMMU second-stage page table configuration (Sv39x4).
// TODO: Add Sv48x4 and Sv57x4 IommuPtConfig variants and select the best
// available mode at init time based on capabilities.SV48X4 / capabilities.SV57X4.
#[derive(Clone, Debug)]
pub struct IommuPtConfig {}

// SAFETY: This implementation is safe because `item_raw_info`,
// `item_into_raw`, `item_from_raw`, and `item_ref_from_raw` are
// correctly implemented with respect to the `Item` and `ItemRef` types.
unsafe impl PageTableConfig for IommuPtConfig {
    // TODO: Full Sv39x4 requires TOP_LEVEL_INDEX_RANGE = 0..2048 (11-bit VPN[2])
    // with a 16 KiB root table. Currently limited to 512 entries (9-bit)
    // because the `PageTable` infrastructure assumes uniform 4 KiB tables.
    const TOP_LEVEL_INDEX_RANGE: Range<usize> = 0..512;

    type E = PageTableEntry;
    type C = PagingConsts;

    type Item = PtItem;
    type ItemRef<'a> = PtItemRef<'a>;

    fn item_raw_info(item: &Self::Item) -> (Paddr, PagingLevel, PageProperty) {
        (item.0, item.1, item.2)
    }

    unsafe fn item_from_raw(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self::Item {
        (paddr, level, prop)
    }

    unsafe fn item_ref_from_raw<'a>(
        _paddr: Paddr,
        _level: PagingLevel,
        _prop: PageProperty,
    ) -> Self::ItemRef<'a> {
        PhantomData
    }
}

pub(crate) type PtItem = (Paddr, PagingLevel, PageProperty);
pub(crate) type PtItemRef<'a> = PhantomData<&'a ()>;

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 3;
    // Reduced from 41 (11+9+9+12) due to the 4 KiB root table limitation above.
    const ADDRESS_WIDTH: usize = 39;
    const VA_SIGN_EXT: bool = false;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 1;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    pub struct PteFlags: usize {
        /// Valid entry.
        const V =           1 << 0;
        /// Readable.
        const R =           1 << 1;
        /// Writable.
        const W =           1 << 2;
        /// Executable.
        const X =           1 << 3;
        /// Accessed.
        const A =           1 << 6;
        /// Dirty.
        const D =           1 << 7;

        // Software-available bits.
        const RSV1 =        1 << 8;
        const RSV2 =        1 << 9;

        /// PBMT: Non-cacheable, idempotent, weakly-ordered (main memory).
        const PBMT_NC =     1 << 61;
        /// PBMT: Non-cacheable, non-idempotent, strongly-ordered (I/O).
        const PBMT_IO =     1 << 62;
    }
}

fn map_flag(val: impl Into<usize>, from: impl Into<usize>, to: impl Into<usize>) -> usize {
    let v: usize = val.into();
    let f: usize = from.into();
    let t: usize = to.into();
    ((v & f) >> f.ilog2()) << t.ilog2()
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub struct PageTableEntry(usize);

impl PageTableEntry {
    const PHYS_MASK: usize = 0x003F_FFFF_FFFF_FC00;

    fn new_without_flags(paddr: Paddr) -> Self {
        Self((paddr >> 12) << 10)
    }

    fn paddr(&self) -> Paddr {
        (self.0 & Self::PHYS_MASK) >> 10 << 12
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        let rwx = PteFlags::R.bits() | PteFlags::W.bits() | PteFlags::X.bits();
        level == 1 || (self.0 & rwx) != 0
    }

    fn prop(&self) -> PageProperty {
        let flags = map_flag(self.0, PteFlags::R.bits(), PageFlags::R.bits())
            | map_flag(self.0, PteFlags::W.bits(), PageFlags::W.bits())
            | map_flag(self.0, PteFlags::X.bits(), PageFlags::X.bits())
            | map_flag(self.0, PteFlags::A.bits(), PageFlags::ACCESSED.bits())
            | map_flag(self.0, PteFlags::D.bits(), PageFlags::DIRTY.bits())
            | map_flag(self.0, PteFlags::RSV2.bits(), PageFlags::AVAIL2.bits());

        let priv_flags = map_flag(self.0, PteFlags::RSV1.bits(), PrivFlags::AVAIL1.bits());

        let cache = if self.0 & PteFlags::PBMT_IO.bits() != 0 {
            CachePolicy::Uncacheable
        } else {
            CachePolicy::Writeback
        };

        PageProperty {
            flags: PageFlags::from_bits(flags as u8).unwrap(),
            cache,
            priv_flags: PrivFlags::from_bits(priv_flags as u8).unwrap(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        let bits = PageTableFlags::empty().bits() as usize
            | map_flag(self.0, PteFlags::RSV1.bits(), PageTableFlags::AVAIL1.bits())
            | map_flag(self.0, PteFlags::RSV2.bits(), PageTableFlags::AVAIL2.bits());
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::V.bits()
            | map_flag(prop.flags.bits(), PageFlags::R.bits(), PteFlags::R.bits())
            | map_flag(prop.flags.bits(), PageFlags::W.bits(), PteFlags::W.bits())
            | map_flag(prop.flags.bits(), PageFlags::X.bits(), PteFlags::X.bits())
            | map_flag(
                prop.flags.bits(),
                PageFlags::ACCESSED.bits(),
                PteFlags::A.bits(),
            )
            | map_flag(
                prop.flags.bits(),
                PageFlags::DIRTY.bits(),
                PteFlags::D.bits(),
            )
            | map_flag(
                prop.priv_flags.bits(),
                PrivFlags::AVAIL1.bits(),
                PteFlags::RSV1.bits(),
            )
            | map_flag(
                prop.flags.bits(),
                PageFlags::AVAIL2.bits(),
                PteFlags::RSV2.bits(),
            );

        match prop.cache {
            CachePolicy::Writeback => (),
            CachePolicy::Uncacheable => {
                flags |= PteFlags::PBMT_IO.bits();
            }
            _ => panic!("unsupported cache policy for IOMMU"),
        }

        Self(Self::new_without_flags(paddr).0 | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let flags = PteFlags::V.bits()
            | map_flag(
                flags.bits(),
                PageTableFlags::AVAIL1.bits(),
                PteFlags::RSV1.bits(),
            )
            | map_flag(
                flags.bits(),
                PageTableFlags::AVAIL2.bits(),
                PteFlags::RSV2.bits(),
            );

        Self(Self::new_without_flags(paddr).0 | flags)
    }
}

impl PodOnce for PageTableEntry {}

// SAFETY: This implementation is safe because `from_repr` and `to_repr`
// correctly round-trip between the `PteScalar` representation and
// the hardware PTE format, and a zeroed PTE represents an absent entry (V=0).
// `from_usize`/`into_usize` are not overridden so the default no-op
// implementations apply.
unsafe impl PteTrait for PageTableEntry {
    fn from_repr(repr: &PteScalar, level: PagingLevel) -> Self {
        match repr {
            PteScalar::Absent => PageTableEntry(0),
            PteScalar::PageTable(paddr, flags) => Self::new_pt(*paddr, *flags),
            PteScalar::Mapped(paddr, prop) => Self::new_page(*paddr, level, *prop),
        }
    }

    fn to_repr(&self, level: PagingLevel) -> PteScalar {
        if self.0 & PteFlags::V.bits() == 0 {
            return PteScalar::Absent;
        }

        if self.is_last(level) {
            PteScalar::Mapped(self.paddr(), self.prop())
        } else {
            PteScalar::PageTable(self.paddr(), self.pt_flags())
        }
    }
}
