use crate::sync::Mutex;
use alloc::{collections::BTreeMap, fmt};
use pod::Pod;
use x86_64::{instructions::tlb, structures::paging::PhysFrame, VirtAddr};

use crate::{
    config::ENTRY_COUNT,
    vm::{
        page_table::{table_of, PageTableEntryTrait, PageTableFlagsTrait},
        Paddr, Vaddr,
    },
};

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
        /// Only in the non-starting and non-ending levels, indication of huge page.
        const HUGE =            1 << 7;
        /// Indicates that the mapping is present in all address spaces, so it isn't flushed from
        /// the TLB on an address space switch.
        const GLOBAL =          1 << 8;
        /// Forbid execute codes on the page. The NXE bits in EFER msr must be set.
        const NO_EXECUTE =      1 << 63;
    }
}

pub fn tlb_flush(vaddr: Vaddr) {
    tlb::flush(VirtAddr::new(vaddr as u64));
}

pub const fn is_user_vaddr(vaddr: Vaddr) -> bool {
    // FIXME: Support 3/5 level page table.
    // 47 = 12(offset) + 4 * 9(index) - 1
    (vaddr >> 47) == 0
}

pub const fn is_kernel_vaddr(vaddr: Vaddr) -> bool {
    // FIXME: Support 3/5 level page table.
    // 47 = 12(offset) + 4 * 9(index) - 1
    ((vaddr >> 47) & 0x1) == 1
}

#[derive(Clone, Copy, Pod)]
#[repr(C)]
pub struct PageTableEntry(usize);

/// ## Safety
///
/// Changing the level 4 page table is unsafe, because it's possible to violate memory safety by
/// changing the page mapping.
pub unsafe fn activate_page_table(root_paddr: Paddr, flags: x86_64::registers::control::Cr3Flags) {
    x86_64::registers::control::Cr3::write(
        PhysFrame::from_start_address(x86_64::PhysAddr::new(root_paddr as u64)).unwrap(),
        flags,
    );
}

pub static ALL_MAPPED_PTE: Mutex<BTreeMap<usize, PageTableEntry>> = Mutex::new(BTreeMap::new());

pub fn init() {
    let (page_directory_base, _) = x86_64::registers::control::Cr3::read();
    let page_directory_base = page_directory_base.start_address().as_u64() as usize;

    // Safety: page_directory_base is read from Cr3, the address is valid.
    let p4 = unsafe { table_of::<PageTableEntry>(page_directory_base).unwrap() };
    // Cancel mapping in lowest addresses.
    p4[0].clear();
    let mut map_pte = ALL_MAPPED_PTE.lock();
    for (i, p4_i) in p4.iter().enumerate().take(512) {
        if p4_i.flags().contains(PageTableFlags::PRESENT) {
            map_pte.insert(i, *p4_i);
        }
    }
}

impl PageTableFlagsTrait for PageTableFlags {
    fn new() -> Self {
        Self::empty()
    }

    fn set_present(mut self, present: bool) -> Self {
        self.set(Self::PRESENT, present);
        self
    }

    fn set_writable(mut self, writable: bool) -> Self {
        self.set(Self::WRITABLE, writable);
        self
    }

    fn set_readable(self, readable: bool) -> Self {
        // do nothing
        self
    }

    fn set_accessible_by_user(mut self, accessible: bool) -> Self {
        self.set(Self::USER, accessible);
        self
    }

    fn is_present(&self) -> bool {
        self.contains(Self::PRESENT)
    }

    fn writable(&self) -> bool {
        self.contains(Self::WRITABLE)
    }

    fn readable(&self) -> bool {
        // always true
        true
    }

    fn accessible_by_user(&self) -> bool {
        self.contains(Self::USER)
    }

    fn set_executable(mut self, executable: bool) -> Self {
        self.set(Self::NO_EXECUTE, !executable);
        self
    }

    fn executable(&self) -> bool {
        !self.contains(Self::NO_EXECUTE)
    }

    fn has_accessed(&self) -> bool {
        self.contains(Self::ACCESSED)
    }

    fn is_dirty(&self) -> bool {
        self.contains(Self::DIRTY)
    }

    fn union(&self, flags: &Self) -> Self {
        (*self).union(*flags)
    }

    fn remove(&mut self, flags: &Self) {
        self.remove(*flags)
    }

    fn insert(&mut self, flags: &Self) {
        self.insert(*flags)
    }

    fn is_huge(&self) -> bool {
        self.contains(Self::HUGE)
    }

    fn set_huge(mut self, huge: bool) -> Self {
        self.set(Self::HUGE, huge);
        self
    }
}

impl PageTableEntry {
    /// 51:12
    const PHYS_ADDR_MASK: usize = 0xF_FFFF_FFFF_F000;
}

impl PageTableEntryTrait for PageTableEntry {
    type F = PageTableFlags;
    fn new(paddr: Paddr, flags: PageTableFlags) -> Self {
        Self((paddr & Self::PHYS_ADDR_MASK) | flags.bits)
    }
    fn paddr(&self) -> Paddr {
        self.0 & Self::PHYS_ADDR_MASK
    }
    fn flags(&self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.0)
    }
    fn is_used(&self) -> bool {
        self.0 != 0
    }

    fn update(&mut self, paddr: Paddr, flags: Self::F) {
        self.0 = (paddr & Self::PHYS_ADDR_MASK) | flags.bits;
    }

    fn clear(&mut self) {
        self.0 = 0;
    }

    fn page_index(va: crate::vm::Vaddr, level: usize) -> usize {
        debug_assert!((1..=5).contains(&level));
        va >> (12 + 9 * (level - 1)) & (ENTRY_COUNT - 1)
    }
}

impl fmt::Debug for PageTableEntry {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        let mut f = f.debug_struct("PageTableEntry");
        f.field("raw", &self.0)
            .field("paddr", &self.paddr())
            .field("flags", &self.flags())
            .finish()
    }
}
