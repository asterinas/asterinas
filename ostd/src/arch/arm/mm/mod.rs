// SPDX-License-Identifier: MPL-2.0

use core::{arch::asm, ops::Range};

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
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 4;
    const PTE_SIZE: usize = size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    /// Possible flags for a page table entry.
    #[repr(C)]
    #[derive(Pod)]
    pub(crate) struct PteFlags: usize {
        /// Specifies whether the mapped frame or page table is valid.
        const VALID =           1 << 0;
        /// Specifies whether the mapping does not point to a huge frame; this bit must also be set
        /// for all the valid last-level entries.
        const NON_HUGE =        1 << 1;
        /// Controls whether accesses from userspace (i.e. EL0) are permitted.
        const USER =            1 << 6;
        /// Controls whether writes to the mapped frames are disallowed.
        const NO_WRITE =        1 << 7;
        /// Whether the memory area represented by this entry is accessed.
        const ACCESSED =        1 << 10;
        /// Indicates that the mapping isn't present in all address spaces, so it is flushed from
        /// the TLB on an address space switch.
        const NON_GLOBAL =      1 << 11;

        /// Whether the memory area represented by this entry is modified.
        const DIRTY =           1 << 51;
        /// Forbid execute codes on the page from kernel space (i.e. EL1).
        const NO_EXECUTE_KERN = 1 << 53;
        /// Forbid execute codes on the page from userspace (i.e. EL0).
        const NO_EXECUTE_USER = 1 << 54;

        /// Ignored by the hardware. Free to use.
        const HIGH_IGN1 =       1 << 55;
        /// Ignored by the hardware. Free to use.
        const HIGH_IGN2 =       1 << 56;

        // Be careful that the following fields contain multiple bits!
        //
        /// Bit 2-4: Device memory, nGnRnE.
        const ATTR_DEVICE =     1 << 2;
        /// Bit 8-9: Inner shareability (effective only for Normal memory).
        const SH_INNER =        3 << 8;
    }
}

const SHARED_ASID: usize = 0;

/// The bit offset of the ASID field in TTBR registers and TLBI operands.
const ASID_SHIFT: u32 = 48;

pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    // SAFETY: This invalidates the TLB, which doesn't affect the memory safety.
    unsafe {
        asm!(
            "dsb nshst",
            "tlbi vaae1, {vpn}",
            "dsb nsh",
            "isb",
            vpn = in(reg) vaddr_to_vpn(vaddr),
        );
    }
}

pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    // SAFETY: This invalidates the TLB, which doesn't affect the memory safety.
    unsafe {
        asm!("dsb nshst");
        for vaddr in range.clone().step_by(PAGE_SIZE) {
            asm!("tlbi vaae1, {vpn}", vpn = in(reg) vaddr_to_vpn(vaddr));
        }
        asm!("dsb nsh", "isb");
    }
}

fn vaddr_to_vpn(vaddr: Vaddr) -> usize {
    // Bits 43-0: Bits[55:12] of the virtual address to match.
    (vaddr >> 12) & ((1 << 44) - 1)
}

pub(crate) fn tlb_flush_all_excluding_global() {
    // We use `SHARED_ASID` all the time, so all non-global pages are associated with it.
    //
    // SAFETY: This invalidates the TLB, which doesn't affect the memory safety.
    unsafe {
        asm!("dsb nshst", "tlbi aside1, {asid}", "dsb nsh", "isb", asid = in(reg) SHARED_ASID << ASID_SHIFT)
    };
}

pub(crate) fn tlb_flush_all_including_global() {
    // SAFETY: This invalidates the TLB, which doesn't affect the memory safety.
    unsafe { asm!("dsb nshst", "tlbi vmalle1", "dsb nsh", "isb") };
}

pub(crate) fn can_sync_dma() -> bool {
    false
}

/// # Safety
///
/// The caller must ensure that
///  - the virtual address range and DMA direction correspond correctly to a
///    DMA region;
///  - `can_sync_dma()` is `true`.
#[expect(clippy::extra_unused_type_parameters)]
pub(crate) unsafe fn sync_dma_range<D: DmaDirection>(_range: Range<Vaddr>) {
    unreachable!("`can_sync_dma()` never returns `true`");
}

/// Activates the given root-level page table.
///
/// # Safety
///
/// Changing the root-level page table is unsafe, because it's possible to violate memory safety by
/// changing the page mapping.
pub(crate) unsafe fn activate_page_table(root_paddr: Paddr) {
    debug_assert_eq!(root_paddr >> ASID_SHIFT, 0);
    let ttbr = root_paddr | (SHARED_ASID << ASID_SHIFT);

    // SAFETY: The safety is upheld by the caller.
    unsafe {
        asm!(
            "msr ttbr0_el1, {ttbr}",
            "msr ttbr1_el1, {ttbr}",
            "isb",
            ttbr = in(reg) ttbr,
        );
    }

    // We reuse `SHARED_ASID`, so we need to flush the TLB entries.
    tlb_flush_all_excluding_global();
}

pub(crate) fn current_page_table_paddr() -> Paddr {
    let ttbr: usize;
    // SAFETY: It is safe to read the register containing the root-level page table address.
    unsafe {
        asm!(
            "mrs {ttbr}, ttbr0_el1",
            ttbr = out(reg) ttbr,
        );
    }

    debug_assert_eq!(ttbr >> ASID_SHIFT, SHARED_ASID);
    ttbr - (SHARED_ASID << ASID_SHIFT)
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Default, Pod)]
pub(crate) struct PageTableEntry(usize);

/// Parses a bit-flag bits `val` in the representation of `from` to `to` in bits.
macro_rules! parse_flags {
    ($val:expr, $from:expr, $to:expr) => {
        (($val as usize & $from.bits() as usize) >> $from.bits().ilog2() << $to.bits().ilog2())
    };
}

impl PageTableEntry {
    const PHYS_ADDR_MASK: usize = 0x0000_FFFF_FFFF_F000;

    fn is_present(&self) -> bool {
        if self.0 & PteFlags::VALID.bits() != 0 {
            // Child page tables and readable pages.
            true
        } else if self.0 & PteFlags::SH_INNER.bits() != 0 {
            // Non-readable pages (`new_page()` always sets `SH_INNER`).
            true
        } else {
            // Nothing.
            false
        }
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1 || self.0 & PteFlags::NON_HUGE.bits() == 0
    }

    fn paddr(&self) -> Paddr {
        self.0 & Self::PHYS_ADDR_MASK
    }

    fn prop(&self) -> PageProperty {
        let mut flags = parse_flags!(self.0, PteFlags::VALID, PageFlags::R)
            | parse_flags!(!self.0, PteFlags::NO_WRITE, PageFlags::W)
            | parse_flags!(self.0, PteFlags::ACCESSED, PageFlags::ACCESSED)
            | parse_flags!(self.0, PteFlags::DIRTY, PageFlags::DIRTY)
            | parse_flags!(self.0, PteFlags::HIGH_IGN2, PageFlags::AVAIL2);
        if self.0 & PteFlags::USER.bits() != 0 {
            flags |= parse_flags!(!self.0, PteFlags::NO_EXECUTE_USER, PageFlags::X);
        } else {
            flags |= parse_flags!(!self.0, PteFlags::NO_EXECUTE_KERN, PageFlags::X);
        }

        let priv_flags = parse_flags!(self.0, PteFlags::USER, PrivFlags::USER)
            | parse_flags!(!self.0, PteFlags::NON_GLOBAL, PrivFlags::GLOBAL)
            | parse_flags!(self.0, PteFlags::HIGH_IGN1, PrivFlags::AVAIL1);

        let cache = if self.0 & PteFlags::ATTR_DEVICE.bits() != 0 {
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
            | parse_flags!(self.0, PteFlags::HIGH_IGN1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::HIGH_IGN2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        // FIXME: To avoid the Access Flag Fault,
        // we set the `ACCESSED` bit to 1 all the time.
        let mut flags = PteFlags::ACCESSED.bits();
        if level == 1 {
            flags |= PteFlags::NON_HUGE.bits();
        }

        flags |= parse_flags!(prop.flags.bits(), PageFlags::R, PteFlags::VALID)
            | parse_flags!(!prop.flags.bits(), PageFlags::W, PteFlags::NO_WRITE)
            | parse_flags!(prop.flags.bits(), PageFlags::ACCESSED, PteFlags::ACCESSED)
            | parse_flags!(prop.flags.bits(), PageFlags::DIRTY, PteFlags::DIRTY)
            | parse_flags!(prop.priv_flags.bits(), PrivFlags::USER, PteFlags::USER)
            | parse_flags!(
                !prop.priv_flags.bits(),
                PrivFlags::GLOBAL,
                PteFlags::NON_GLOBAL
            )
            | parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::AVAIL1,
                PteFlags::HIGH_IGN1
            )
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::HIGH_IGN2);
        if prop.priv_flags.contains(PrivFlags::USER) {
            flags |= PteFlags::NO_EXECUTE_KERN.bits()
                | parse_flags!(!prop.flags.bits(), PageFlags::X, PteFlags::NO_EXECUTE_USER);
        } else {
            flags |= PteFlags::NO_EXECUTE_USER.bits()
                | parse_flags!(!prop.flags.bits(), PageFlags::X, PteFlags::NO_EXECUTE_KERN);
        }

        flags |= PteFlags::SH_INNER.bits();
        match prop.cache {
            CachePolicy::Writeback => (),
            CachePolicy::Uncacheable => {
                // TODO: Currently Asterinas uses `Uncacheable` only for I/O
                // memory. Normal memory can also be `Noncacheable`, where the
                // attribute should not be set to `ATTR_DEVICE`.
                flags |= PteFlags::ATTR_DEVICE.bits();
            }
            _ => panic!("unsupported cache policy"),
        }

        debug_assert_eq!(
            paddr & !Self::PHYS_ADDR_MASK,
            0,
            "page physical address contains invalid bits"
        );
        Self(paddr | flags)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let flags = PteFlags::VALID.bits()
            | PteFlags::NON_HUGE.bits()
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL1, PteFlags::HIGH_IGN1)
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL2, PteFlags::HIGH_IGN2);

        debug_assert_eq!(
            paddr & !Self::PHYS_ADDR_MASK,
            0,
            "page table physical address contains invalid bits"
        );
        Self(paddr | flags)
    }
}

impl PodOnce for PageTableEntry {}

// SAFETY: The implementation is safe because:
//  - `from_usize` and `into_usize` are not overridden;
//  - `from_repr` and `repr` are correctly implemented;
//  - a zeroed PTE represents an absent entry.
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

        if self.is_last(level) {
            PteScalar::Mapped(self.paddr(), self.prop())
        } else {
            PteScalar::PageTable(self.paddr(), self.pt_flags())
        }
    }
}
