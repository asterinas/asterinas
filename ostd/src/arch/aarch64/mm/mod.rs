// SPDX-License-Identifier: MPL-2.0

//! Page table entry and memory management for AArch64 (VMSAv8-64, 4 KiB
//! granule, 4-level, 48-bit virtual addresses).

use core::ops::Range;

pub(crate) use util::{
    __atomic_cmpxchg_fallible, __atomic_load_fallible, __memcpy_fallible, __memset_fallible,
};

use crate::mm::{
    PAGE_SIZE, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr,
    dma::DmaDirection,
    page_prop::{
        CachePolicy, PageFlags, PageProperty, PageTableFlags, PrivilegedPageFlags as PrivFlags,
    },
    page_table::{PteScalar, PteTrait},
};

mod util;

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    // Blocks are permitted at AArch64 translation levels 1 (1 GiB) and 2
    // (2 MiB) but not level 0. In Asterinas numbering (level 1 = base page,
    // level 4 = root), that is level 3 and below.
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 3;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

/// The MAIR attribute index for Normal write-back memory.
const MAIR_IDX_NORMAL: usize = 0;
/// The MAIR attribute index for Device-nGnRnE memory.
const MAIR_IDX_DEVICE: usize = 1;

// Descriptor bit positions (VMSAv8-64 stage-1, 4 KiB granule).
const PTE_VALID: usize = 1 << 0;
/// At levels 0-2: 1 = table, 0 = block. At level 3: must be 1 for a page.
const PTE_TABLE_OR_PAGE: usize = 1 << 1;
const PTE_ATTR_INDX_SHIFT: usize = 2;
const PTE_AP_EL0: usize = 1 << 6; // AP[1]: allow EL0 access
const PTE_AP_RO: usize = 1 << 7; // AP[2]: read-only
const PTE_SH_INNER: usize = 0b11 << 8;
const PTE_AF: usize = 1 << 10; // access flag
const PTE_NG: usize = 1 << 11; // not global
const PTE_PXN: usize = 1 << 53; // privileged execute never
const PTE_UXN: usize = 1 << 54; // unprivileged execute never
// Software-reserved bits [58:55] used to carry Asterinas metadata.
const PTE_SW_DIRTY: usize = 1 << 55;
const PTE_SW_PG_AVAIL1: usize = 1 << 56;
const PTE_SW_PG_AVAIL2: usize = 1 << 57;
const PTE_SW_PRIV_AVAIL1: usize = 1 << 58;

const PTE_ADDR_MASK: usize = 0x0000_ffff_ffff_f000;

fn tlbi_barrier_before() {
    // SAFETY: A data-synchronization barrier has no memory-safety implications.
    unsafe { core::arch::asm!("dsb ishst", options(nostack, preserves_flags)) };
}

fn tlbi_barrier_after() {
    // SAFETY: Barriers have no memory-safety implications.
    unsafe { core::arch::asm!("dsb ish", "isb", options(nostack, preserves_flags)) };
}

pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    tlbi_barrier_before();
    // `tlbi vaae1is` invalidates by VA for all ASIDs, inner-shareable.
    // SAFETY: Invalidating the TLB is always safe.
    unsafe {
        core::arch::asm!(
            "tlbi vaae1is, {}",
            in(reg) (vaddr >> 12) & 0xffff_ffff_ffff,
            options(nostack, preserves_flags),
        )
    };
    tlbi_barrier_after();
}

pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    for vaddr in range.clone().step_by(PAGE_SIZE) {
        tlb_flush_addr(vaddr);
    }
}

pub(crate) fn tlb_flush_all_excluding_global() {
    tlbi_barrier_before();
    // SAFETY: Invalidating the TLB is always safe.
    unsafe { core::arch::asm!("tlbi vmalle1is", options(nostack, preserves_flags)) };
    tlbi_barrier_after();
}

pub(crate) fn tlb_flush_all_including_global() {
    tlbi_barrier_before();
    // `vmalle1is` invalidates all stage-1 EL1&0 entries (including global) for
    // the current VMID, inner-shareable. (`alle1is` is an EL2-only operation.)
    // SAFETY: Invalidating the TLB is always safe.
    unsafe { core::arch::asm!("tlbi vmalle1is", options(nostack, preserves_flags)) };
    tlbi_barrier_after();
}

pub(crate) fn can_sync_dma() -> bool {
    // QEMU `virt` presents coherent DMA; cache maintenance is a no-op for now.
    false
}

/// # Safety
///
/// The caller must ensure that the virtual address range and DMA direction
/// correspond correctly to a DMA region and that `can_sync_dma()` is `true`.
pub(crate) unsafe fn sync_dma_range<D: DmaDirection>(_range: Range<Vaddr>) {
    // TODO: Implement `DC CVAC`/`DC IVAC` cache maintenance for non-coherent
    // DMA. Currently unreachable because `can_sync_dma()` returns `false`.
    unreachable!("cache maintenance for non-coherent DMA is not implemented");
}

/// Activates the given root-level page table.
///
/// On AArch64 the low (user) half is translated through `TTBR0_EL1` and the
/// high (kernel) half through `TTBR1_EL1`. Asterinas maintains a single 512-entry
/// root whose low indices describe user space and high indices describe kernel
/// space, so we point both translation-table base registers at it.
///
/// # Safety
///
/// Changing the root-level page table can violate memory safety by changing the
/// page mapping.
pub(crate) unsafe fn activate_page_table(root_paddr: Paddr, _root_pt_cache: CachePolicy) {
    assert!(root_paddr.is_multiple_of(PagingConsts::BASE_PAGE_SIZE));

    // SAFETY: The caller guarantees that `root_paddr` refers to a valid root
    // page table describing a memory-safe address space.
    unsafe {
        core::arch::asm!(
            "msr ttbr0_el1, {root}",
            "msr ttbr1_el1, {root}",
            "dsb ish",
            "tlbi vmalle1is",
            "dsb ish",
            "isb",
            root = in(reg) root_paddr,
            options(nostack, preserves_flags),
        )
    };
}

pub(crate) fn current_page_table_paddr() -> Paddr {
    let ttbr0: usize;
    // SAFETY: Reading `TTBR0_EL1` has no side effects.
    unsafe { core::arch::asm!("mrs {}, ttbr0_el1", out(reg) ttbr0, options(nostack, nomem)) };
    ttbr0 & PTE_ADDR_MASK
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct PageTableEntry(usize);

impl PageTableEntry {
    fn paddr(&self) -> Paddr {
        self.0 & PTE_ADDR_MASK
    }

    /// Whether this entry is a leaf (block/page) rather than a next-level table.
    fn is_last(&self, level: PagingLevel) -> bool {
        // Level 1 (base page, AArch64 L3): a valid entry is always a page.
        // Higher levels: bit 1 clear means a block (leaf); set means a table.
        level == 1 || (self.0 & PTE_TABLE_OR_PAGE) == 0
    }

    fn prop(&self) -> PageProperty {
        let raw = self.0;

        let is_user = (raw & PTE_AP_EL0) != 0;
        let writable = (raw & PTE_AP_RO) == 0;
        let executable = if is_user {
            (raw & PTE_UXN) == 0
        } else {
            (raw & PTE_PXN) == 0
        };

        let mut flags = PageFlags::R;
        if writable {
            flags |= PageFlags::W;
        }
        if executable {
            flags |= PageFlags::X;
        }
        if (raw & PTE_AF) != 0 {
            flags |= PageFlags::ACCESSED;
        }
        if (raw & PTE_SW_DIRTY) != 0 {
            flags |= PageFlags::DIRTY;
        }
        if (raw & PTE_SW_PG_AVAIL2) != 0 {
            flags |= PageFlags::AVAIL2;
        }

        let mut priv_flags = PrivFlags::empty();
        if is_user {
            priv_flags |= PrivFlags::USER;
        }
        if (raw & PTE_NG) == 0 {
            priv_flags |= PrivFlags::GLOBAL;
        }
        if (raw & PTE_SW_PRIV_AVAIL1) != 0 {
            priv_flags |= PrivFlags::AVAIL1;
        }

        let attr_indx = (raw >> PTE_ATTR_INDX_SHIFT) & 0b111;
        let cache = if attr_indx == MAIR_IDX_DEVICE {
            CachePolicy::Uncacheable
        } else {
            CachePolicy::Writeback
        };

        PageProperty {
            flags,
            cache,
            priv_flags,
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        let mut flags = PageTableFlags::empty();
        if (self.0 & PTE_SW_PG_AVAIL1) != 0 {
            flags |= PageTableFlags::AVAIL1;
        }
        if (self.0 & PTE_SW_PG_AVAIL2) != 0 {
            flags |= PageTableFlags::AVAIL2;
        }
        flags
    }

    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        let mut raw = (paddr & PTE_ADDR_MASK) | PTE_VALID | PTE_AF;

        // Level 1 (AArch64 L3) leaves are pages and must set bit 1; blocks at
        // higher levels leave it clear.
        if level == 1 {
            raw |= PTE_TABLE_OR_PAGE;
        }

        let is_user = prop.priv_flags.contains(PrivFlags::USER);
        let executable = prop.flags.contains(PageFlags::X);

        if !prop.flags.contains(PageFlags::W) {
            raw |= PTE_AP_RO;
        }
        if is_user {
            raw |= PTE_AP_EL0;
        }

        // Execute-never bits: forbid execution wherever it is not requested.
        if is_user {
            raw |= PTE_PXN; // never executable at EL1
            if !executable {
                raw |= PTE_UXN;
            }
        } else {
            raw |= PTE_UXN; // never executable at EL0
            if !executable {
                raw |= PTE_PXN;
            }
        }

        if !prop.priv_flags.contains(PrivFlags::GLOBAL) {
            raw |= PTE_NG;
        }
        if prop.flags.contains(PageFlags::DIRTY) {
            raw |= PTE_SW_DIRTY;
        }
        if prop.flags.contains(PageFlags::AVAIL2) {
            raw |= PTE_SW_PG_AVAIL2;
        }
        if prop.priv_flags.contains(PrivFlags::AVAIL1) {
            raw |= PTE_SW_PRIV_AVAIL1;
        }

        match prop.cache {
            CachePolicy::Writeback => {
                raw |= MAIR_IDX_NORMAL << PTE_ATTR_INDX_SHIFT;
                raw |= PTE_SH_INNER;
            }
            CachePolicy::Uncacheable => {
                raw |= MAIR_IDX_DEVICE << PTE_ATTR_INDX_SHIFT;
                // Shareability is ignored for Device memory.
            }
            _ => panic!("unsupported cache policy"),
        }

        Self(raw)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let mut raw = (paddr & PTE_ADDR_MASK) | PTE_VALID | PTE_TABLE_OR_PAGE;
        if flags.contains(PageTableFlags::AVAIL1) {
            raw |= PTE_SW_PG_AVAIL1;
        }
        if flags.contains(PageTableFlags::AVAIL2) {
            raw |= PTE_SW_PG_AVAIL2;
        }
        Self(raw)
    }
}

impl PodOnce for PageTableEntry {}

// SAFETY: The implementation is correct because:
//  - `from_usize`/`into_usize` are not overridden;
//  - `from_repr`/`to_repr` are inverse operations at a given level;
//  - a zeroed PTE (bit 0 clear) represents an absent entry.
unsafe impl PteTrait for PageTableEntry {
    fn from_repr(repr: &PteScalar, level: PagingLevel) -> Self {
        match repr {
            PteScalar::Absent => PageTableEntry(0),
            PteScalar::PageTable(paddr, flags) => Self::new_pt(*paddr, *flags),
            PteScalar::Mapped(paddr, prop) => Self::new_page(*paddr, level, *prop),
        }
    }

    fn to_repr(&self, level: PagingLevel) -> PteScalar {
        if self.0 & PTE_VALID == 0 {
            return PteScalar::Absent;
        }

        if self.is_last(level) {
            PteScalar::Mapped(self.paddr(), self.prop())
        } else {
            PteScalar::PageTable(self.paddr(), self.pt_flags())
        }
    }
}
