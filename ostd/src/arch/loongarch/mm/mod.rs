// SPDX-License-Identifier: MPL-2.0

use core::{arch::asm, intrinsics::AtomicOrdering::Relaxed, ops::Range};

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
    #[derive(Pod)]
    #[repr(C)]
    /// Possible flags for a page table entry.
    pub(crate) struct PteFlags: usize {
        /// Specifies whether the mapped frame is valid.
        const VALID =           1 << 0;
        /// Whether the memory area represented by this entry is modified.
        const DIRTY =           1 << 1;
        /// Privilege level corresponding to the page table entry.
        /// When `RPLV` = 0, the page table entry can be accessed by any program
        /// with a privilege level not lower than `PLV`;
        /// When `RPLV` = 1, this page table entry can only be accessed by programs
        /// with privilege level equal to `PLV`.
        const PLVL =            1 << 2;
        const PLVH =            1 << 3;
        /// Controls the memory access type of the memory access operation
        /// falling on the address space of the table page entry.
        const MATL =            1 << 4;
        const MATH =            1 << 5;
        /// If this entry is a basic page table entry, it is `GLOBAL`,
        /// which means that the mapping is present in all address spaces,
        /// so it isn't flushed from the TLB on an address space switch.
        /// If this entry is a huge page table entry, it is `HUGE`,
        /// which means that the memory area represented by this entry is
        /// a huge page.
        const GLOBAL_OR_HUGE =  1 << 6;
        /// Specifies whether the mapped frame or page table is loaded in memory.
        /// This flag does not fill in TLB.
        const PRESENT =         1 << 7;
        /// Controls whether writes to the mapped frames are allowed.
        /// This flag does not fill in TLB.
        const WRITABLE =        1 << 8;
        // Whether this entry is a basic page table entry.
        const IS_BASIC =        1 << 9;
        // First bit ignored by MMU.
        const RSV1 =            1 << 10;
        // Second bit ignored by MMU.
        const RSV2 =            1 << 11;
        /// If this entry is a huge page table entry, it is `GLOBAL`.
        const GLOBAL_IN_HUGE =  1 << 12;
        /// Controls whether reads to the mapped frames are not allowed.
        const NOT_READABLE =    1 << 61;
        /// Controls whether execution code in the mapped frames are not allowed.
        const NOT_EXECUTABLE =  1 << 62;
        /// Whether the `PageTableEntry` can only be accessed by the privileged level `PLV` field inferred
        const RPLV =            1 << 63;
    }
}

pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    unsafe {
        asm!(
            "invtlb 0, $zero, {}",
            in(reg) vaddr
        );
    }
}

pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    for vaddr in range.clone().step_by(PAGE_SIZE) {
        tlb_flush_addr(vaddr);
    }
}

pub(crate) fn tlb_flush_all_excluding_global() {
    unsafe {
        asm!("invtlb 3, $zero, $zero");
    }
}

pub(crate) fn tlb_flush_all_including_global() {
    unsafe {
        asm!("invtlb 0, $zero, $zero");
    }
}

pub(crate) fn can_sync_dma() -> bool {
    // TODO: Implement DMA synchronization for LoongArch64 architecture.
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
/// "pgdl" or "pgdh" register doesn't have a field that encodes the cache policy,
/// so `_root_pt_cache` is ignored.
///
/// # Safety
///
/// Changing the root-level page table is unsafe, because it's possible to violate memory safety by
/// changing the page mapping.
pub(crate) unsafe fn activate_page_table(root_paddr: Paddr, _root_pt_cache: CachePolicy) {
    assert!(root_paddr.is_multiple_of(PagingConsts::BASE_PAGE_SIZE));
    loongArch64::register::pgdl::set_base(root_paddr);
    loongArch64::register::pgdh::set_base(root_paddr);
}

pub(crate) fn current_page_table_paddr() -> Paddr {
    let pgdl = loongArch64::register::pgdl::read().raw();
    let pgdh = loongArch64::register::pgdh::read().raw();
    assert_eq!(
        pgdl, pgdh,
        "Only support to share the same page table for both user and kernel space"
    );
    pgdl
}

#[derive(Debug, Clone, Copy, Pod, Default)]
#[repr(C)]
pub(crate) struct PageTableEntry(usize);

/// Parses a bit-flag bits `val` in the representation of `from` to `to` in bits.
macro_rules! parse_flags {
    ($val:expr, $from:expr, $to:expr) => {
        (($val as usize & $from.bits() as usize) >> $from.bits().ilog2() << $to.bits().ilog2())
    };
}

impl PageTableEntry {
    const PHYS_ADDR_MASK: usize = 0x0000_FFFF_FFFF_F000;

    fn is_user(&self) -> bool {
        self.0 & PteFlags::PLVL.bits() != 0 && self.0 & PteFlags::PLVH.bits() != 0
    }

    fn is_huge(&self) -> bool {
        if self.0 & PteFlags::IS_BASIC.bits() != 0 {
            false
        } else {
            self.0 & PteFlags::GLOBAL_OR_HUGE.bits() != 0
        }
    }

    fn is_global(&self) -> bool {
        if self.0 & PteFlags::IS_BASIC.bits() != 0 {
            self.0 & PteFlags::GLOBAL_OR_HUGE.bits() != 0
        } else {
            self.0 & PteFlags::GLOBAL_IN_HUGE.bits() != 0
        }
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        level == 1 || self.is_huge()
    }

    fn paddr(&self) -> Paddr {
        if self.is_huge() {
            let paddr = (self.0 & Self::PHYS_ADDR_MASK & !PteFlags::GLOBAL_IN_HUGE.bits()) >> 12;
            paddr << 12
        } else {
            let ppn = (self.0 & Self::PHYS_ADDR_MASK) >> 12;
            ppn << 12
        }
    }

    fn prop(&self) -> PageProperty {
        let flags = parse_flags!(!(self.0), PteFlags::NOT_READABLE, PageFlags::R)
            | parse_flags!(self.0, PteFlags::WRITABLE, PageFlags::W)
            | parse_flags!(!(self.0), PteFlags::NOT_EXECUTABLE, PageFlags::X)
            // TODO: How to get the accessed bit in loongarch?
            | parse_flags!(self.0, PteFlags::PRESENT, PageFlags::ACCESSED)
            | parse_flags!(self.0, PteFlags::DIRTY, PageFlags::DIRTY)
            | parse_flags!(self.0, PteFlags::RSV2, PageFlags::AVAIL2);

        let mut priv_flags = parse_flags!(self.0, PteFlags::RSV1, PrivFlags::AVAIL1);
        if self.is_user() {
            priv_flags |= PrivFlags::USER.bits() as usize;
        }
        if self.is_global() {
            priv_flags |= PrivFlags::GLOBAL.bits() as usize;
        }

        let cache = if self.0 & PteFlags::MATL.bits() != 0 {
            CachePolicy::Writeback
        } else if self.0 & PteFlags::MATH.bits() != 0 {
            CachePolicy::WriteCombining
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
            | parse_flags!(self.0, PteFlags::RSV1, PageTableFlags::AVAIL1)
            | parse_flags!(self.0, PteFlags::RSV2, PageTableFlags::AVAIL2);
        PageTableFlags::from_bits(bits as u8).unwrap()
    }

    fn new_page(paddr: Paddr, level: PagingLevel, prop: PageProperty) -> Self {
        let mut flags = PteFlags::VALID.bits()
            // FIXME: To avoid the PageModifyFault exception,
            // we set the DIRTY bit to 1 all the time.
            | PteFlags::DIRTY.bits()
            | parse_flags!(
                !prop.flags.bits(),
                PageFlags::R,
                PteFlags::NOT_READABLE
            )
            | parse_flags!(prop.flags.bits(), PageFlags::W, PteFlags::WRITABLE)
            | parse_flags!(
                !prop.flags.bits(),
                PageFlags::X,
                PteFlags::NOT_EXECUTABLE
            )
            | parse_flags!(prop.flags.bits(), PageFlags::DIRTY, PteFlags::DIRTY)
            // TODO: How to get the accessed bit in loongarch?
            | parse_flags!(prop.flags.bits(), PageFlags::ACCESSED, PteFlags::PRESENT)
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PteFlags::RSV2);
        flags |= parse_flags!(prop.priv_flags.bits(), PrivFlags::AVAIL1, PteFlags::RSV1);
        if prop.priv_flags.contains(PrivFlags::USER) {
            flags |= PteFlags::PLVL.bits();
            flags |= PteFlags::PLVH.bits();
        }
        if prop.priv_flags.contains(PrivFlags::GLOBAL) {
            if level != 1 {
                flags |= PteFlags::GLOBAL_IN_HUGE.bits();
            } else {
                flags |= PteFlags::GLOBAL_OR_HUGE.bits();
            }
        }
        match prop.cache {
            CachePolicy::Writeback => {
                flags |= PteFlags::MATL.bits();
            }
            CachePolicy::Uncacheable => (),
            CachePolicy::WriteCombining => {
                flags |= PteFlags::MATH.bits();
            }
            _ => panic!("unsupported cache policy"),
        }
        let level_bits = if level != 1 {
            PteFlags::GLOBAL_OR_HUGE.bits()
        } else {
            PteFlags::IS_BASIC.bits()
        };
        Self((paddr & Self::PHYS_ADDR_MASK) | flags | level_bits)
    }

    fn new_pt(paddr: Paddr, flags: PageTableFlags) -> Self {
        let flags = PteFlags::VALID.bits()
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL1, PteFlags::RSV1)
            | parse_flags!(flags.bits(), PageTableFlags::AVAIL2, PteFlags::RSV2);
        Self(paddr & Self::PHYS_ADDR_MASK | flags)
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
    const DEVICE_LINEAR_MAPPING_BASE_VADDR: usize = 0x8000_0000_0000_0000;
    pa + DEVICE_LINEAR_MAPPING_BASE_VADDR
}

pub(crate) unsafe fn __memcpy_fallible(dst: *mut u8, src: *const u8, size: usize) -> usize {
    // TODO: Implement this fallible operation.
    unsafe { core::ptr::copy(src, dst, size) };
    0
}

pub(crate) unsafe fn __memset_fallible(dst: *mut u8, value: u8, size: usize) -> usize {
    // TODO: Implement this fallible operation.
    unsafe { core::ptr::write_bytes(dst, value, size) };
    0
}

pub(crate) unsafe fn __atomic_load_fallible(ptr: *const u32) -> u64 {
    // TODO: Implement this fallible operation.
    unsafe { core::intrinsics::atomic_load::<_, { Relaxed }>(ptr) as u64 }
}

pub(crate) unsafe fn __atomic_cmpxchg_fallible(ptr: *mut u32, old_val: u32, new_val: u32) -> u64 {
    // TODO: Implement this fallible operation.
    unsafe {
        core::intrinsics::atomic_cxchg::<_, { Relaxed }, { Relaxed }>(ptr, old_val, new_val).0
            as u64
    }
}
