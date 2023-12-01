use pod::Pod;

use crate::{
    config::ENTRY_COUNT,
    vm::page_table::{PageTableEntryTrait, PageTableFlagsTrait},
};

bitflags::bitflags! {
    #[derive(Pod)]
    #[repr(C)]
    pub struct PageTableFlags : u64{
        /// Whether accesses to this page must snoop processor caches.
        const SNOOP =           1 << 11;

        const DIRTY =           1 << 9;

        const ACCESSED =        1 << 8;
        /// Whether this page table entry is the last entry.
        const LAST_PAGE =       1 << 7;

        /// Ignore PAT, 1 if the scalable-mode PASID-table entry is not
        /// used for effective memory-type determination.
        const IGNORE_PAT =     1 << 6;

        /// Extended Memory Type, ignored by hardware when the
        /// Extended Memory Type Enable (EMTE) field is Clear.
        ///
        /// When the EMTE field is Set, this field is used to compute effective
        /// memory-type for second-stage-only and nested translations.
        const EMT =             7 << 3;

        const WRITABLE =        1 << 1;

        const READABLE =        1 << 0;

    }
}

#[derive(Debug, Clone, Copy, Pod)]
#[repr(C)]
pub struct PageTableEntry(u64);

impl PageTableFlagsTrait for PageTableFlags {
    fn new() -> Self {
        Self::empty()
    }

    fn set_present(self, present: bool) -> Self {
        self
    }

    fn set_writable(mut self, writable: bool) -> Self {
        self.set(Self::WRITABLE, writable);
        self
    }

    fn set_readable(mut self, readable: bool) -> Self {
        self.set(Self::READABLE, readable);
        self
    }

    fn set_accessible_by_user(self, accessible: bool) -> Self {
        self
    }

    fn set_executable(self, executable: bool) -> Self {
        self
    }

    fn is_present(&self) -> bool {
        self.contains(Self::WRITABLE) || self.contains(Self::READABLE)
    }

    fn writable(&self) -> bool {
        self.contains(Self::WRITABLE)
    }

    fn readable(&self) -> bool {
        self.contains(Self::READABLE)
    }

    fn executable(&self) -> bool {
        true
    }

    fn has_accessed(&self) -> bool {
        self.contains(Self::ACCESSED)
    }

    fn is_dirty(&self) -> bool {
        self.contains(Self::DIRTY)
    }

    fn accessible_by_user(&self) -> bool {
        true
    }

    fn union(&self, other: &Self) -> Self {
        (*self).union(*other)
    }

    fn remove(&mut self, flags: &Self) {
        self.remove(*flags)
    }

    fn insert(&mut self, flags: &Self) {
        self.insert(*flags)
    }

    fn is_huge(&self) -> bool {
        // FIXME: this is super bad
        false
    }

    fn set_huge(self, huge: bool) -> Self {
        // FIXME: this is super bad
        self
    }
}

impl PageTableEntry {
    const PHYS_MASK: usize = 0xFFFF_FFFF_F000;
}

impl PageTableEntryTrait for PageTableEntry {
    // bit 47~12
    type F = PageTableFlags;
    fn new(paddr: crate::vm::Paddr, flags: PageTableFlags) -> Self {
        Self((paddr & Self::PHYS_MASK) as u64 | flags.bits)
    }

    fn paddr(&self) -> crate::vm::Paddr {
        (self.0 & Self::PHYS_MASK as u64) as usize
    }

    fn flags(&self) -> PageTableFlags {
        PageTableFlags::from_bits_truncate(self.0)
    }

    fn is_used(&self) -> bool {
        self.paddr() != 0
    }

    fn update(&mut self, paddr: crate::vm::Paddr, flags: Self::F) {
        self.0 = (paddr & Self::PHYS_MASK) as u64 | flags.bits
    }

    fn clear(&mut self) {
        self.0 = 0;
    }

    fn page_index(va: crate::vm::Vaddr, level: usize) -> usize {
        debug_assert!((1..=5).contains(&level));
        va >> (12 + 9 * (level - 1)) & (ENTRY_COUNT - 1)
    }
}
