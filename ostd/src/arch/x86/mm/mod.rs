// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use cfg_if::cfg_if;
pub(crate) use util::{
    __atomic_cmpxchg_fallible, __atomic_load_fallible, __memcpy_fallible, __memset_fallible,
};
use x86_64::{
    VirtAddr,
    instructions::tlb,
    registers::model_specific::{Efer, EferFlags},
    structures::paging::PhysFrame,
};

use crate::mm::{
    PAGE_SIZE, Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr,
    dma::DmaDirection,
    page_prop::{
        CachePolicy, PageFlags, PageProperty, PageTableFlags, PrivilegedPageFlags as PrivFlags,
    },
    page_table::{PteScalar, PteTrait},
};

mod pat;
mod util;

use self::pat::{cache_policy_to_flags, configure_pat, flags_to_cache_policy};

#[derive(Clone, Debug, Default)]
pub(crate) struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 2;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    /// Possible flags for a page table entry.
    pub(crate) struct PteFlags: usize {
        /// Specifies whether the mapped frame or page table is loaded in memory.
        const PRESENT =         1 << 0;
        /// Controls whether writes to the mapped frames are allowed.
        const WRITABLE =        1 << 1;
        /// Controls whether accesses from userspace (i.e. ring 3) are permitted.
        const USER =            1 << 2;
        /// If this bit is set, a “write-through” policy is used for the cache, else a “write-back”
        /// policy is used.
        const WRITE_THROUGH =   1 << 3;
        /// Disables caching for the pointed entry is cacheable.
        const NO_CACHE =        1 << 4;
        /// Whether this entry has been used for linear-address translation.
        const ACCESSED =        1 << 5;
        /// Whether the memory area represented by this entry is modified.
        const DIRTY =           1 << 6;
        /// In level 2 or 3 it indicates that it map to a huge page.
        /// In level 1, it is the PAT (page attribute table) bit.
        /// We use this bit in level 1, 2 and 3 to indicate that this entry is
        /// "valid". For levels above 3, `PRESENT` is used for "valid".
        const HUGE =            1 << 7;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 8;
        /// TDX shared bit.
        #[cfg(feature = "cvm_guest")]
        const SHARED =          1 << 51;

        /// Ignored by the hardware. Free to use.
        const HIGH_IGN1 =       1 << 52;
        /// Ignored by the hardware. Free to use.
        const HIGH_IGN2 =       1 << 53;

        /// Forbid execute codes on the page. The NXE bits in EFER msr must be set.
        const NO_EXECUTE =      1 << 63;
    }
}

/// Flush any TLB entry that contains the map of the given virtual address.
///
/// This flush performs regardless of the global-page bit. So it can flush both global
/// and non-global entries.
pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    tlb::flush(VirtAddr::new(vaddr as u64));
}

/// Flush any TLB entry that intersects with the given address range.
pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    for vaddr in range.clone().step_by(PAGE_SIZE) {
        tlb_flush_addr(vaddr);
    }
}

/// Flush all TLB entries except for the global-page entries.
pub(crate) fn tlb_flush_all_excluding_global() {
    tlb::flush_all();
}

/// Flush all TLB entries, including global-page entries.
pub(crate) fn tlb_flush_all_including_global() {
    // SAFETY: updates to CR4 here only change the global-page bit, the side effect
    // is only to invalidate the TLB, which doesn't affect the memory safety.
    unsafe {
        // To invalidate all entries, including global-page
        // entries, disable global-page extensions (CR4.PGE=0).
        x86_64::registers::control::Cr4::update(|cr4| {
            *cr4 -= x86_64::registers::control::Cr4Flags::PAGE_GLOBAL;
        });
        x86_64::registers::control::Cr4::update(|cr4| {
            *cr4 |= x86_64::registers::control::Cr4Flags::PAGE_GLOBAL;
        });
    }
}

pub(crate) fn can_sync_dma() -> bool {
    true
}

/// # Safety
///
/// The caller must ensure that
///  - the virtual address range and DMA direction correspond correctly to a
///    DMA region;
///  - `can_sync_dma()` is `true`.
pub(crate) unsafe fn sync_dma_range<D: DmaDirection>(_range: Range<Vaddr>) {
    // The streaming DMA mapping in x86_64 is cache coherent, and does not
    // require synchronization.
    // Reference: <https://lwn.net/Articles/855328/>, <https://lwn.net/Articles/2265/>.
}

/// Activates the given root-level page table.
///
/// The cache policy of the root page table node is controlled by `root_pt_cache`.
///
/// # Safety
///
/// Changing the root-level page table is unsafe, because it's possible to violate memory safety by
/// changing the page mapping.
pub(crate) unsafe fn activate_page_table(root_paddr: Paddr, root_pt_cache: CachePolicy) {
    let addr = PhysFrame::from_start_address(x86_64::PhysAddr::new(root_paddr as u64)).unwrap();
    let flags = match root_pt_cache {
        CachePolicy::Writeback => x86_64::registers::control::Cr3Flags::empty(),
        CachePolicy::Writethrough => x86_64::registers::control::Cr3Flags::PAGE_LEVEL_WRITETHROUGH,
        CachePolicy::Uncacheable => x86_64::registers::control::Cr3Flags::PAGE_LEVEL_CACHE_DISABLE,
        // Write-combining and write-protected are not supported for root page table (CR3)
        // as CR3 only supports WB, WT, and UC via PCD/PWT bits
        _ => {
            panic!(
                "unsupported cache policy for the root page table (only WB, WT, and UC are allowed)"
            )
        }
    };

    // SAFETY: The safety is upheld by the caller.
    unsafe { x86_64::registers::control::Cr3::write(addr, flags) };
}

pub(crate) fn current_page_table_paddr() -> Paddr {
    x86_64::registers::control::Cr3::read_raw()
        .0
        .start_address()
        .as_u64() as Paddr
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub(crate) struct PageTableEntry(usize);

/// Parses a bit-flag bits `val` in the representation of `from` to `to` in bits.
macro_rules! parse_flags {
    ($val:expr, $from:expr, $to:expr) => {
        (($val as usize & $from.bits() as usize) >> $from.bits().ilog2() << $to.bits().ilog2())
    };
}

impl PageTableEntry {
    cfg_if! {
        if #[cfg(feature = "cvm_guest")] {
            const PHYS_ADDR_MASK_LVL1: usize = 0x7_ffff_ffff_f000;
            const PHYS_ADDR_MASK_LVL2: usize = 0x7_ffff_ffe0_0000;
            const PHYS_ADDR_MASK_LVL3: usize = 0x7_ffff_c000_0000;
        } else {
            const PHYS_ADDR_MASK_LVL1: usize = 0xf_ffff_ffff_f000;
            const PHYS_ADDR_MASK_LVL2: usize = 0xf_ffff_ffe0_0000;
            const PHYS_ADDR_MASK_LVL3: usize = 0xf_ffff_c000_0000;
        }
    }

    const CHILD_PT_ADDR_MASK: usize = Self::PHYS_ADDR_MASK_LVL1;

    fn pa_mask_at_level(level: PagingLevel) -> usize {
        match level {
            1 => Self::PHYS_ADDR_MASK_LVL1,
            2 => Self::PHYS_ADDR_MASK_LVL2,
            3 => Self::PHYS_ADDR_MASK_LVL3,
            _ => panic!("invalid level {} for page entry", level),
        }
    }

    fn is_present(&self) -> bool {
        // For PT child, `PRESENT` should be set; for huge page, `HUGE` should
        // be set; for the leaf child page, `PAT`, which is the same bit as
        // the `HUGE` bit in upper levels, should be set.
        self.0 & PteFlags::PRESENT.bits() != 0 || self.0 & PteFlags::HUGE.bits() != 0
    }

    fn prop(&self) -> PageProperty {
        let flags = parse_flags!(self.0, PteFlags::PRESENT, PageFlags::R)
            | parse_flags!(self.0, PteFlags::WRITABLE, PageFlags::W)
            | parse_flags!(!self.0, PteFlags::NO_EXECUTE, PageFlags::X)
            | parse_flags!(self.0, PteFlags::ACCESSED, PageFlags::ACCESSED)
            | parse_flags!(self.0, PteFlags::DIRTY, PageFlags::DIRTY)
            | parse_flags!(self.0, PteFlags::HIGH_IGN2, PageFlags::AVAIL2);

        let priv_flags = parse_flags!(self.0, PteFlags::USER, PrivFlags::USER)
            | parse_flags!(self.0, PteFlags::GLOBAL, PrivFlags::GLOBAL)
            | parse_flags!(self.0, PteFlags::HIGH_IGN1, PrivFlags::AVAIL1);

        #[cfg(feature = "cvm_guest")]
        let priv_flags = priv_flags | parse_flags!(self.0, PteFlags::SHARED, PrivFlags::SHARED);

        // Determine cache policy from PCD, PWT bits.
        let cache = flags_to_cache_policy(PteFlags::from_bits_truncate(self.0));

        PageProperty {
            flags: PageFlags::from_bits(flags as u8).unwrap(),
            cache,
            priv_flags: PrivFlags::from_bits(priv_flags as u8).unwrap(),
        }
    }

    fn pt_flags(&self) -> PageTableFlags {
        let bits = PageTableFlags::empty().bits() as usize
            | parse_flags!(self.0, PteFlags::HIGH_IGN1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::HIGH_IGN2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::HUGE.bits();

        flags |= parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::PRESENT)
            | parse_flags!(prop.flags.bits(), PageFlags::W, PteFlags::WRITABLE)
            | parse_flags!(!prop.flags.bits(), PageFlags::X, PteFlags::NO_EXECUTE)
            | parse_flags!(prop.flags.bits(), PageFlags::ACCESSED, PteFlags::ACCESSED)
            | parse_flags!(prop.flags.bits(), PageFlags::DIRTY, PteFlags::DIRTY)
            | parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::AVAIL1,
                PteFlags::HIGH_IGN1
            )
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::HIGH_IGN2)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::USER, PteFlags::USER)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::GLOBAL, PteFlags::GLOBAL);
        #[cfg(feature = "cvm_guest")]
        {
            flags |= parse_flags!(prop.priv_flags.bits(), PrivFlags::SHARED, PteFlags::SHARED);
        }

        flags |= cache_policy_to_flags(prop.cache).bits();

        assert_eq!(
            paddr & !Self::pa_mask_at_level(level),
            0,
            "page physical address contains invalid bits"
        );
        Self(paddr | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        // In x86 if it's an intermediate PTE, it's better to have the same permissions
        // as the most permissive child (to reduce hardware page walk accesses). But we
        // don't have a mechanism to keep it generic across architectures, thus just
        // setting it to be the most permissive.
        let flags = PteFlags::PRESENT.bits()
            | PteFlags::WRITABLE.bits()
            | PteFlags::USER.bits()
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL1, PteFlags::HIGH_IGN1)
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL2, PteFlags::HIGH_IGN2);

        assert_eq!(
            paddr & !Self::CHILD_PT_ADDR_MASK,
            0,
            "page table physical address contains invalid bits"
        );
        Self(paddr | flags)
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

        if self.0 & PteFlags::HUGE.bits() != 0 {
            let paddr = self.0 & Self::pa_mask_at_level(level);
            PteScalar::Mapped(paddr, self.prop())
        } else {
            let paddr = self.0 & Self::CHILD_PT_ADDR_MASK;
            PteScalar::PageTable(paddr, self.pt_flags())
        }
    }
}

/// Enables memory-management essential features for the x86 MMU.
pub(super) fn enable_essential_features() {
    // Page Attribute Table (PAT) has been available since Pentium III (1999)
    // and is ubiquitous in modern 64-bit CPUs. Therefore, we assume that all
    // x86-64 CPUs should have PAT support. Otherwise, we should check
    // `cpu::extension::has_extensions(IsaExtensions::PAT)` before programming it.
    configure_pat();

    unsafe {
        // Enable non-executable page protection.
        Efer::update(|efer| {
            *efer |= EferFlags::NO_EXECUTE_ENABLE;
        });
    }
}
