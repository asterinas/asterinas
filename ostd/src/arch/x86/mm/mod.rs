// SPDX-License-Identifier: MPL-2.0

#![expect(dead_code)]

use alloc::fmt;
use core::ops::Range;

use cfg_if::cfg_if;
pub(crate) use util::{
    __atomic_cmpxchg_fallible, __atomic_load_fallible, __memcpy_fallible, __memset_fallible,
};
use x86_64::{instructions::tlb, structures::paging::PhysFrame, VirtAddr};

use crate::{
    mm::{
        page_prop::{CachePolicy, PageFlags, PageProperty, PrivilegedPageFlags as PrivFlags},
        page_table::PageTableEntryTrait,
        Paddr, PagingConstsTrait, PagingLevel, PodOnce, Vaddr, PAGE_SIZE,
    },
    Pod,
};

mod util;

pub(crate) const NR_ENTRIES_PER_PAGE: usize = 512;

#[derive(Clone, Debug, Default)]
pub struct PagingConsts {}

impl PagingConstsTrait for PagingConsts {
    const BASE_PAGE_SIZE: usize = 4096;
    const NR_LEVELS: PagingLevel = 4;
    const ADDRESS_WIDTH: usize = 48;
    const VA_SIGN_EXT: bool = true;
    const HIGHEST_TRANSLATION_LEVEL: PagingLevel = 2;
    const PTE_SIZE: usize = core::mem::size_of::<PageTableEntry>();
}

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    /// Possible flags for a page table entry.
    pub struct PageTableFlags: usize {
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

#[derive(Clone, Copy, Pod, Default)]
#[repr(C)]
pub struct PageTableEntry(usize);

/// Activates the given level 4 page table.
/// The cache policy of the root page table node is controlled by `root_pt_cache`.
///
/// # Safety
///
/// Changing the level 4 page table is unsafe, because it's possible to violate memory safety by
/// changing the page mapping.
pub unsafe fn activate_page_table(root_paddr: Paddr, root_pt_cache: CachePolicy) {
    let addr = PhysFrame::from_start_address(x86_64::PhysAddr::new(root_paddr as u64)).unwrap();
    let flags = match root_pt_cache {
        CachePolicy::Writeback => x86_64::registers::control::Cr3Flags::empty(),
        CachePolicy::Writethrough => x86_64::registers::control::Cr3Flags::PAGE_LEVEL_WRITETHROUGH,
        CachePolicy::Uncacheable => x86_64::registers::control::Cr3Flags::PAGE_LEVEL_CACHE_DISABLE,
        _ => panic!("unsupported cache policy for the root page table"),
    };

    // SAFETY: The safety is upheld by the caller.
    unsafe { x86_64::registers::control::Cr3::write(addr, flags) };
}

pub fn current_page_table_paddr() -> Paddr {
    x86_64::registers::control::Cr3::read_raw()
        .0
        .start_address()
        .as_u64() as Paddr
}

impl PageTableEntry {
    cfg_if! {
        if #[cfg(feature = "cvm_guest")] {
            const PHYS_ADDR_MASK: usize = 0x7_FFFF_FFFF_F000;
        } else {
            const PHYS_ADDR_MASK: usize = 0xF_FFFF_FFFF_F000;
        }
    }
    const PROP_MASK: usize = !Self::PHYS_ADDR_MASK & !PageTableFlags::HUGE.bits();
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
        // For PT child, `PRESENT` should be set; for huge page, `HUGE` should
        // be set; for the leaf child page, `PAT`, which is the same bit as
        // the `HUGE` bit in upper levels, should be set.
        self.0 & PageTableFlags::PRESENT.bits() != 0 || self.0 & PageTableFlags::HUGE.bits() != 0
    }

    fn new_page(paddr: Paddr, _level: PagingLevel, prop: PageProperty) -> Self {
        let flags = PageTableFlags::HUGE.bits();
        let mut pte = Self(paddr & Self::PHYS_ADDR_MASK | flags);
        pte.set_prop(prop);
        pte
    }

    fn new_pt(paddr: Paddr) -> Self {
        // In x86 if it's an intermediate PTE, it's better to have the same permissions
        // as the most permissive child (to reduce hardware page walk accesses). But we
        // don't have a mechanism to keep it generic across architectures, thus just
        // setting it to be the most permissive.
        let flags = PageTableFlags::PRESENT.bits()
            | PageTableFlags::WRITABLE.bits()
            | PageTableFlags::USER.bits();
        Self(paddr & Self::PHYS_ADDR_MASK | flags)
    }

    fn paddr(&self) -> Paddr {
        self.0 & Self::PHYS_ADDR_MASK
    }

    fn prop(&self) -> PageProperty {
        let flags = (parse_flags!(self.0, PageTableFlags::PRESENT, PageFlags::R))
            | (parse_flags!(self.0, PageTableFlags::WRITABLE, PageFlags::W))
            | (parse_flags!(!self.0, PageTableFlags::NO_EXECUTE, PageFlags::X))
            | (parse_flags!(self.0, PageTableFlags::ACCESSED, PageFlags::ACCESSED))
            | (parse_flags!(self.0, PageTableFlags::DIRTY, PageFlags::DIRTY))
            | (parse_flags!(self.0, PageTableFlags::HIGH_IGN2, PageFlags::AVAIL2));
        let priv_flags = (parse_flags!(self.0, PageTableFlags::USER, PrivFlags::USER))
            | (parse_flags!(self.0, PageTableFlags::GLOBAL, PrivFlags::GLOBAL))
            | (parse_flags!(self.0, PageTableFlags::HIGH_IGN1, PrivFlags::AVAIL1));
        #[cfg(feature = "cvm_guest")]
        let priv_flags =
            priv_flags | (parse_flags!(self.0, PageTableFlags::SHARED, PrivFlags::SHARED));
        let cache = if self.0 & PageTableFlags::NO_CACHE.bits() != 0 {
            CachePolicy::Uncacheable
        } else if self.0 & PageTableFlags::WRITE_THROUGH.bits() != 0 {
            CachePolicy::Writethrough
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
        if !self.is_present() {
            return;
        }
        let mut flags = PageTableFlags::empty().bits();
        flags |= (parse_flags!(prop.flags.bits(), PageFlags::R, PageTableFlags::PRESENT))
            | (parse_flags!(prop.flags.bits(), PageFlags::W, PageTableFlags::WRITABLE))
            | (parse_flags!(!prop.flags.bits(), PageFlags::X, PageTableFlags::NO_EXECUTE))
            | (parse_flags!(
                prop.flags.bits(),
                PageFlags::ACCESSED,
                PageTableFlags::ACCESSED
            ))
            | (parse_flags!(prop.flags.bits(), PageFlags::DIRTY, PageTableFlags::DIRTY))
            | (parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::AVAIL1,
                PageTableFlags::HIGH_IGN1
            ))
            | (parse_flags!(
                prop.flags.bits(),
                PageFlags::AVAIL2,
                PageTableFlags::HIGH_IGN2
            ))
            | (parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::USER,
                PageTableFlags::USER
            ))
            | (parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::GLOBAL,
                PageTableFlags::GLOBAL
            ));
        #[cfg(feature = "cvm_guest")]
        {
            flags |= parse_flags!(
                prop.priv_flags.bits(),
                PrivFlags::SHARED,
                PageTableFlags::SHARED
            );
        }
        match prop.cache {
            CachePolicy::Writeback => {}
            CachePolicy::Writethrough => {
                flags |= PageTableFlags::WRITE_THROUGH.bits();
            }
            CachePolicy::Uncacheable => {
                flags |= PageTableFlags::NO_CACHE.bits();
            }
            _ => panic!("unsupported cache policy"),
        }
        self.0 = self.0 & !Self::PROP_MASK | flags;
    }

    fn is_last(&self, _level: PagingLevel) -> bool {
        self.0 & PageTableFlags::HUGE.bits() != 0
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
