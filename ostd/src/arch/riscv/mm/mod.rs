// SPDX-License-Identifier: MPL-2.0

use alloc::fmt;
use core::ops::Range;

use crate::{
    cpu::extension::{has_extensions, IsaExtensions},
    mm::{
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags as PrivFlags},
        page_table::PageTableEntryTrait,
        Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr, PAGE_SIZE,
    },
    Pod,
};

pub(crate) const NR_ENTRIES_PER_PAGE: usize = 512;

#[derive(Clone, Debug, Default)]
pub struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 4;
    const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    /// Possible flags for a page table entry.
    pub struct PageTableFlags: usize {
        /// Specifies whether the mapped frame or page table is valid.
        const VALID =           1 << 0;
        /// Controls whether reads to the mapped frames are allowed.
        const READABLE =        1 << 1;
        /// Controls whether writes to the mapped frames are allowed.
        const WRITABLE =        1 << 2;
        /// Controls whether execution code in the mapped frames are allowed.
        const EXECUTABLE =      1 << 3;
        /// Controls whether accesses from userspace (i.e. U-mode) are permitted.
        const USER =            1 << 4;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 5;
        /// Whether the memory area represented by this entry is accessed.
        const ACCESSED =        1 << 6;
        /// Whether the memory area represented by this entry is modified.
        const DIRTY =           1 << 7;

        // First bit ignored by MMU.
        const RSV1 =            1 << 8;
        // Second bit ignored by MMU.
        const RSV2 =            1 << 9;

        // PBMT: Non-cacheable, idempotent, weakly-ordered (RVWMO), main memory
        const PBMT_NC =         1 << 61;
        // PBMT: Non-cacheable, non-idempotent, strongly-ordered (I/O ordering), I/O
        const PBMT_IO =         1 << 62;
        /// Naturally aligned power-of-2
        const NAPOT =           1 << 63;
    }
}

pub(crate) fn tlb_flush_addr(vaddr: Vaddr) {
    unsafe {
        riscv::asm::sfence_vma(0, vaddr);
    }
}

pub(crate) fn tlb_flush_addr_range(range: &Range<Vaddr>) {
    for vaddr in range.clone().step_by(PAGE_SIZE) {
        tlb_flush_addr(vaddr);
    }
}

pub(crate) fn tlb_flush_all_excluding_global() {
    // TODO: excluding global?
    riscv::asm::sfence_vma_all()
}

pub(crate) fn tlb_flush_all_including_global() {
    riscv::asm::sfence_vma_all()
}

#[derive(Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct PageTableEntry(usize);

/// Activate the given level 4 page table.
///
/// "satp" register doesn't have a field that encodes the cache policy,
/// so `_root_pt_cache` is ignored.
///
/// # Safety
///
/// Changing the level 4 page table is unsafe, because it's possible to violate memory safety by
/// changing the page mapping.
pub unsafe fn activate_page_table(root_paddr: Paddr, _root_pt_cache: CachePolicy) {
    assert!(root_paddr % PagingConsts::BASE_PAGE_SIZE == 0);
    let ppn = root_paddr >> 12;
    riscv::register::satp::set(riscv::register::satp::Mode::Sv48, 0, ppn);
}

pub fn current_page_table_paddr() -> Paddr {
    riscv::register::satp::read().ppn() << 12
}

impl PageTableEntry {
    const PHYS_ADDR_MASK: usize = 0x003F_FFFF_FFFF_FC00;

    fn new_paddr(paddr: Paddr) -> Self {
        let ppn = paddr >> 12;
        Self(ppn << 10)
    }
}

/// Parse a bit-flag bits `val` in the representation of `from` to `to` in bits.
macro_rules! parse_flags {
    ($val:expr, $from:expr, $to:expr) => {
        ($val as usize & $from.bits() as usize) >> $from.bits().ilog2() << $to.bits().ilog2()
    };
}

impl PodOnce for PageTableEntry {}

impl PageTableEntryTrait for PageTableEntry {
    fn is_present(&self) -> bool {
        self.0 & PageTableFlags::VALID.bits() != 0
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let mut pte = Self::new_paddr(paddr);
        pte.set_prop(prop);
        pte
    }

    fn new_pt(paddr: Paddr) -> Self {
        // In RISC-V, non-leaf PTE should have RWX = 000,
        // and D, A, and U are reserved for future standard use.
        let pte = Self::new_paddr(paddr);
        PageTableEntry(pte.0 | PageTableFlags::VALID.bits())
    }

    fn paddr(&self) -> Paddr {
        let ppn = (self.0 & Self::PHYS_ADDR_MASK) >> 10;
        ppn << 12
    }

    fn prop(&self) -> PageProperty {
        let flags = (parse_flags!(self.0, PageTableFlags::READABLE, PageFlags::R))
            | (parse_flags!(self.0, PageTableFlags::WRITABLE, PageFlags::W))
            | (parse_flags!(self.0, PageTableFlags::EXECUTABLE, PageFlags::X))
            | (parse_flags!(self.0, PageTableFlags::ACCESSED, PageFlags::ACCESSED))
            | (parse_flags!(self.0, PageTableFlags::DIRTY, PageFlags::DIRTY))
            | (parse_flags!(self.0, PageTableFlags::RSV2, PageFlags::AVAIL2));
        let priv_flags = (parse_flags!(self.0, PageTableFlags::USER, PrivFlags::USER))
            | (parse_flags!(self.0, PageTableFlags::GLOBAL, PrivFlags::GLOBAL))
            | (parse_flags!(self.0, PageTableFlags::RSV1, PrivFlags::AVAIL1));

        let cache = if self.0 & PageTableFlags::PBMT_IO.bits() != 0 {
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

    fn set_prop(&mut self, prop: PageProperty) {
        let mut flags = PageTableFlags::VALID.bits()
            | parse_flags!(prop.flags.bits(), PageFlags::R, PageTableFlags::READABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::W, PageTableFlags::WRITABLE)
            | parse_flags!(prop.flags.bits(), PageFlags::X, PageTableFlags::EXECUTABLE)
            | parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::USER,
                PageTableFlags::USER
            )
            | parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::GLOBAL,
                PageTableFlags::GLOBAL
            )
            | parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::AVAIL1,
                PageTableFlags::RSV1
            )
            | parse_flags!(prop.flags.bits(), PageFlags::AVAIL2, PageTableFlags::RSV2);

        match prop.cache {
            CachePolicy::Writeback => (),
            CachePolicy::Uncacheable => {
                // TODO: Currently Asterinas uses `Uncacheable` only for I/O
                // memory. Normal memory can also be `Noncacheable`, where the
                // PBMT should be set to `PBMT_NC`.
                if has_extensions(IsaExtensions::SVPBMT) {
                    flags |= PageTableFlags::PBMT_IO.bits()
                }
            }
            _ => panic!("unsupported cache policy"),
        }

        self.0 = (self.0 & Self::PHYS_ADDR_MASK) | flags;
    }

    fn is_last(&self, level: PagingLevel) -> bool {
        let rwx = PageTableFlags::READABLE | PageTableFlags::WRITABLE | PageTableFlags::EXECUTABLE;
        level == 1 || (self.0 & rwx.bits()) != 0
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut f = f.debug_struct("PageTableEntry");
        f.field("raw", &format_args!("{:#x}", self.0))
            .field("paddr", &format_args!("{:#x}", self.paddr()))
            .field("present", &self.is_present())
            .field(
                "flags",
                &PageTableFlags::from_bits_truncate(self.0 & !Self::PHYS_ADDR_MASK),
            )
            .field("prop", &self.prop())
            .finish()
    }
}

pub(crate) unsafe fn __memcpy_fallible(dst: *mut u8, src: *const u8, size: usize) -> usize {
    // TODO: Implement this fallible operation.
    unsafe { riscv::register::sstatus::set_sum() };
    unsafe { core::ptr::copy(src, dst, size) };
    0
}

pub(crate) unsafe fn __memset_fallible(dst: *mut u8, value: u8, size: usize) -> usize {
    // TODO: Implement this fallible operation.
    unsafe { riscv::register::sstatus::set_sum() };
    unsafe { core::ptr::write_bytes(dst, value, size) };
    0
}

pub(crate) unsafe fn __atomic_load_fallible(ptr: *const u32) -> u64 {
    // TODO: Implement this fallible operation.
    unsafe { riscv::register::sstatus::set_sum() };
    unsafe { core::intrinsics::atomic_load_relaxed(ptr) as u64 }
}

pub(crate) unsafe fn __atomic_cmpxchg_fallible(ptr: *mut u32, old_val: u32, new_val: u32) -> u64 {
    // TODO: Implement this fallible operation.
    unsafe { riscv::register::sstatus::set_sum() };
    unsafe { core::intrinsics::atomic_cxchg_relaxed_relaxed(ptr, old_val, new_val).0 as u64 }
}
