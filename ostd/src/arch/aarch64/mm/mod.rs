// SPDX-License-Identifier: MPL-2.0

use core::{arch::global_asm, ops::Range};

global_asm!(include_str!("memcpy_fallible.S"));
global_asm!(include_str!("memset_fallible.S"));
global_asm!(include_str!("atomic_load_fallible.S"));
global_asm!(include_str!("atomic_cmpxchg_fallible.S"));

use crate::mm::{
    PAGE_SIZE, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr,
    dma::DmaDirection,
    page_prop::{
        CachePolicy, PageFlags, PageProperty, PageTableFlags, PrivilegedPageFlags as PrivFlags,
    },
    page_table::{PteScalar, PteTrait},
};

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    // TODO: Support huge page
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 1;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[repr(C)]
    #[derive(Pod)]
    /// Possible flags for a page table entry on ARM64.
    pub(crate) struct PteFlags: usize {
        /// Valid bit - entry is present.
        const VALID =           1 << 0;
        /// Block descriptor (when at level 2/3).
        const BLOCK_OR_TABLE =  1 << 1;
        /// Upper attributes (for block/page descriptors).
        const UPPER_ATTRS =     0x0000_0000_0000_0ff0;
        /// Access flag.
        const AF =              1 << 10;
        /// Not global.
        const NG =              1 << 11;
        /// Execution never at EL0.
        const PXN =             1 << 53;
        /// Privileged execute never.
        const UXN =             1 << 54;
        /// Software-defined bits (RES0 bit 55 for all descriptor types).
        const SW =              1 << 55;
    }
}

/// ARM64 page table entry.
#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct PageTableEntry(usize);

impl PageTableEntry {
    const PHYS_ADDR_MASK: usize = 0x0000_FFFF_FFFF_F000;

    fn is_user(&self) -> bool {
        // AP[1] == 0, AP[0] == 1: EL0 read/write, EL1 no access
        // AP[1] == 1, AP[0] == 1: EL0/EL1 read only
        const AP0: usize = 1 << 6;
        const AP1: usize = 1 << 7;
        self.0 & AP0 != 0
    }

    fn is_huge(&self) -> bool {
        self.0 & PteFlags::BLOCK_OR_TABLE.bits() == 0
    }

    fn is_global(&self) -> bool {
        self.0 & PteFlags::NG.bits() == 0
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1 || self.is_huge()
    }

    fn paddr(&self) -> Paddr {
        (self.0 & Self::PHYS_ADDR_MASK) >> 12 << 12
    }

    fn prop(&self) -> PageProperty {
        const AP0: usize = 1 << 6;
        const AP1: usize = 1 << 7;
        const XN: usize = 1 << 54;
        const PXN: usize = 1 << 53;
        const AF: usize = 1 << 10;

        let flags = if self.0 & AP0 != 0 && self.0 & AP1 == 0 {
            // RW at EL1, RO at EL0
            PageFlags::R | PageFlags::W
        } else if self.0 & AP0 != 0 && self.0 & AP1 != 0 {
            // RO at EL1 and EL0
            PageFlags::R
        } else {
            PageFlags::empty()
        };
        let flags = if self.0 & XN != 0 {
            flags
        } else {
            flags | PageFlags::X
        };
        let flags = if self.0 & AF != 0 {
            flags | PageFlags::ACCESSED
        } else {
            flags
        };
        // TODO: DIRTY, AVAIL flags

        let mut priv_flags = PageTableFlags::empty().bits() as usize;
        if self.is_user() {
            priv_flags |= PrivFlags::USER.bits() as usize;
        }
        if self.is_global() {
            priv_flags |= PrivFlags::GLOBAL.bits() as usize;
        }

        PageProperty {
            flags,
            cache: CachePolicy::Writeback,
            priv_flags: PrivFlags::from_bits(priv_flags as u8).unwrap(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        // RES0 bit 55 stores AVAIL1 (PTE_POINTS_TO_FIRMWARE_PT).
        if self.0 & PteFlags::SW.bits() != 0 {
            let mut flags = PageTableFlags::empty();
            flags |= PageTableFlags::AVAIL1;
            flags
        } else {
            PageTableFlags::empty()
        }
    }

    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::VALID.bits() | PteFlags::AF.bits();
        if !prop.priv_flags.contains(PrivFlags::USER) {
            flags |= PteFlags::UXN.bits();
        }
        if !prop.flags.contains(PageFlags::X) {
            flags |= PteFlags::PXN.bits();
        }
        // AP[1:0]: 01 = RW/NA, 11 = RO/RO
        if !prop.flags.contains(PageFlags::W) {
            flags |= 1 << 6; // AP0
            flags |= 1 << 7; // AP1
        } else if prop.priv_flags.contains(PrivFlags::USER) {
            flags |= 1 << 6; // AP0
        }
        if !prop.priv_flags.contains(PrivFlags::GLOBAL) {
            flags |= PteFlags::NG.bits();
        }
        // AttrIndx (bits[4:2]): selects memory type from MAIR_EL1.
        // Attr0=Device, Attr2=Normal cacheable.
        // SH (bits[9:8]): 3 = inner shareable.
        let attr_idx: usize = match prop.cache {
            CachePolicy::Uncacheable | CachePolicy::WriteCombining => 0,
            CachePolicy::Writeback | CachePolicy::WriteProtected | CachePolicy::Writethrough => 2,
        };
        flags |= attr_idx << 2;
        flags |= 3 << 8; // SH = inner shareable
        if level == 1 {
            // Level 1 = finest translation level (4KB page or L2 table entry).
            // Page/table descriptors need bits[1:0] = 0b11.
            flags |= PteFlags::BLOCK_OR_TABLE.bits();
        } else if level != 3 && !is_huge_from_prop(prop) {
            // Table descriptor at intermediate level.
            // bits[1:0] = 0b11: VALID | BLOCK_OR_TABLE.
            flags |= PteFlags::BLOCK_OR_TABLE.bits();
        } else if level != 3 {
            // Block descriptor at intermediate level (1GB or 2MB).
            // bits[1:0] = 0b01: VALID only, NO BLOCK_OR_TABLE.
        }
        Self((paddr & Self::PHYS_ADDR_MASK) | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        // Store AVAIL1 in RES0 bit 55 (PTE_POINTS_TO_FIRMWARE_PT).
        let sw = if flags.contains(PageTableFlags::AVAIL1) {
            PteFlags::SW.bits()
        } else {
            0
        };
        // AttrIndx=2 (Normal cacheable) for page table walk, SH=3 (inner shareable).
        const ATTR_IDX_SH: usize = (2 << 2) | (3 << 8);
        Self(
            (paddr & Self::PHYS_ADDR_MASK)
                | PteFlags::VALID.bits()
                | PteFlags::BLOCK_OR_TABLE.bits()
                | ATTR_IDX_SH
                | PteFlags::AF.bits()
                | sw,
        )
    }
}

fn is_huge_from_prop(_prop: PageProperty) -> bool {
    // TODO: Support huge pages
    false
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
        if self.0 & PteFlags::VALID.bits() == 0 {
            return PteScalar::Absent;
        }

        if self.is_last(level) {
            PteScalar::Mapped(self.paddr(), self.prop())
        } else {
            PteScalar::PageTable(self.paddr(), self.pt_flags())
        }
    }
}

pub(in crate::arch) fn paddr_to_daddr(pa: Paddr) -> usize {
    // TODO: Define device linear mapping base for ARM64
    const DEVICE_LINEAR_MAPPING_BASE_VADDR: usize = 0x8000_0000_0000_0000;
    pa + DEVICE_LINEAR_MAPPING_BASE_VADDR
}

unsafe extern "C" {
    /// Copies `size` bytes from `src` to `dst` with exception handling.
    /// Returns number of bytes that failed to copy.
    pub(crate) fn __memcpy_fallible(dst: *mut u8, src: *const u8, size: usize) -> usize;
    /// Fills `size` bytes at `dst` with `value` with exception handling.
    /// Returns number of bytes that failed to set.
    pub(crate) fn __memset_fallible(dst: *mut u8, value: u8, size: usize) -> usize;
    /// Atomically loads a 32-bit integer with exception handling.
    /// Returns the loaded value or `!0u64` if failed to load.
    pub(crate) fn __atomic_load_fallible(ptr: *const u32) -> u64;
    /// Atomically compares and exchanges a 32-bit integer with exception handling.
    /// Returns the previous value or `!0u64` if failed to update.
    pub(crate) fn __atomic_cmpxchg_fallible(ptr: *mut u32, old_val: u32, new_val: u32) -> u64;
}

pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    // SAFETY: TLBI instructions are always safe to execute.
    unsafe {
        core::arch::asm!(
            "dsb ishst",
            "tlbi vaae1is, {0}",
            "dsb ish",
            "isb",
            in(reg) vaddr,
        );
    }
}

pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    for vaddr in range.clone().step_by(PAGE_SIZE) {
        tlb_flush_addr(vaddr);
    }
}

pub(crate) fn tlb_flush_all_excluding_global() {
    // SAFETY: TLBI instructions are always safe to execute.
    // TLBI VMALLE1IS is the EL1-usable variant that flushes all EL1/EL0 TLB entries.
    // Note: TLBI ALLE1IS is EL2-only and causes an undefined instruction trap at EL1.
    // The semantic difference between excluding/including global is not relevant here
    // since there are no global kernel mappings to preserve at the point this is called.
    unsafe {
        core::arch::asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb",);
    }
}

pub(crate) fn tlb_flush_all_including_global() {
    // SAFETY: TLBI instructions are always safe to execute.
    unsafe {
        core::arch::asm!("dsb ishst", "tlbi vmalle1is", "dsb ish", "isb",);
    }
}

pub(crate) unsafe fn activate_page_table(root_paddr: Paddr, _root_pt_cache: CachePolicy) {
    assert!(root_paddr.is_multiple_of(PagingConsts::BASE_PAGE_SIZE));
    // SAFETY: The caller ensures the root page table is properly initialized.
    // Write both TTBR0_EL1 (user VA, bit 47 = 0) and TTBR1_EL1 (kernel VA,
    // bit 47 = 1). The shared page table design means a single root serves both
    // halves: L0[0..256] for user, L0[256..512] for kernel.
    unsafe {
        core::arch::asm!(
            "msr ttbr0_el1, {0}",
            "msr ttbr1_el1, {0}",
            "isb",
            in(reg) root_paddr,
        );
    }
    tlb_flush_all_including_global();
}

pub(crate) fn current_page_table_paddr() -> Paddr {
    let ttbr1: usize;
    // SAFETY: Reading TTBR1_EL1 is always safe and returns the physical
    // address of the current kernel page table root.
    unsafe { core::arch::asm!("mrs {0}, ttbr1_el1", out(reg) ttbr1) };
    ttbr1 & PageTableEntry::PHYS_ADDR_MASK
}

pub(crate) fn can_sync_dma() -> bool {
    false
}

#[expect(clippy::extra_unused_type_parameters)]
pub(crate) unsafe fn sync_dma_range<D: DmaDirection>(_range: Range<Vaddr>) {
    unreachable!("`can_sync_dma()` never returns `true`");
}
