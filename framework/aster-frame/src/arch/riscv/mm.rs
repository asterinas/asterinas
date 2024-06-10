// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use alloc::fmt;

use pod::Pod;

use crate::vm::{
    page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags as PrivFlags},
    page_table::PageTableEntryTrait,
    Paddr, PagingConstsTrait, Vaddr, PAGE_SIZE,
};

pub(crate) const NR_ENTRIES_PER_PAGE: usize = 512;

#[derive(Debug)]
pub struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: usize = 4;
    const HIGHEST_TRANSLATION_LEVEL: usize = 4;
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

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct PageTableEntry(usize);

/// Activate the given level 4 page table.
///
/// "satp" register doesn't have a field that encodes the cache policy,
/// so `_root_pt_cache` is ignored.
///
/// ## Safety
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
}

impl PageTableEntryTrait for PageTableEntry {
    fn new_absent() -> Self {
        Self(0)
    }

    fn is_present(&self) -> bool {
        self.0 & PageTableFlags::VALID.bits() != 0
    }

    fn new(paddr: Paddr, prop: PageProperty, huge: bool, last: bool) -> Self {
        let mut flags = PageTableFlags::VALID;
        if huge || last {
            if prop.flags.contains(PageFlags::R) {
                flags |= PageTableFlags::READABLE;
            }
            if prop.flags.contains(PageFlags::W) {
                flags |= PageTableFlags::WRITABLE;
            }
            if prop.flags.contains(PageFlags::X) {
                flags |= PageTableFlags::EXECUTABLE;
            }
            if prop.priv_flags.contains(PrivFlags::USER) {
                flags |= PageTableFlags::USER;
            }
            if prop.priv_flags.contains(PrivFlags::GLOBAL) {
                flags |= PageTableFlags::GLOBAL;
            }
        } else {
            // In RISC-V, non-leaf PTE should have RWX = 000,
            // and D, A, and U are reserved for future standard use.
        }

        match prop.cache {
            CachePolicy::Writeback => (),
            CachePolicy::Uncacheable => {
                // Currently, Asterinas uses `Uncacheable` for I/O memory.
                flags |= PageTableFlags::PBMT_IO
            },
            _ => panic!("unsupported cache policy"),
        }

        let ppn = paddr >> 12;
        Self((ppn << 10) | flags.bits())
    }

    fn paddr(&self) -> Paddr {
        let ppn = (self.0 & Self::PHYS_ADDR_MASK) >> 10;
        ppn << 12
    }

    fn prop(&self) -> PageProperty {
        let mut flags = PageFlags::empty();
        let mut priv_flags = PrivFlags::empty();
        if self.0 & PageTableFlags::READABLE.bits() != 0 {
            flags |= PageFlags::R;
        }
        if self.0 & PageTableFlags::WRITABLE.bits() != 0 {
            flags |= PageFlags::W;
        }
        if self.0 & PageTableFlags::EXECUTABLE.bits() != 0 {
            flags |= PageFlags::X;
        }
        if self.0 & PageTableFlags::ACCESSED.bits() != 0 {
            flags |= PageFlags::ACCESSED;
        }
        if self.0 & PageTableFlags::DIRTY.bits() != 0 {
            flags |= PageFlags::DIRTY;
        }

        if self.0 & PageTableFlags::USER.bits() != 0 {
            priv_flags |= PrivFlags::USER;
        }
        if self.0 & PageTableFlags::GLOBAL.bits() != 0 {
            priv_flags |= PrivFlags::GLOBAL;
        }

        let cache = if self.0 & PageTableFlags::PBMT_IO.bits() != 0 {
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

    fn is_huge(&self) -> bool {
        let rwx = PageTableFlags::READABLE | PageTableFlags::WRITABLE | PageTableFlags::EXECUTABLE;
        (self.0 & rwx.bits()) != 0
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
