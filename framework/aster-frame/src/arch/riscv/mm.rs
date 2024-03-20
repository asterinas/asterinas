// SPDX-License-Identifier: MPL-2.0

use core::{arch::asm, fmt};

use bitflags::bitflags;

use crate::vm::{
    page_table::{PageTableEntryTrait, PageTableFlagsTrait},
    Paddr, Vaddr,
};

pub const NR_ENTRIES_PER_PAGE: usize = 512;

bitflags! {
    #[derive(PartialEq)]
    pub struct PageTableFlags: u8 {
        const V = 1 << 0;
        const R = 1 << 1;
        const W = 1 << 2;
        const X = 1 << 3;
        const U = 1 << 4;
        const G = 1 << 5;
        const A = 1 << 6;
        const D = 1 << 7;
    }
}

impl PageTableFlagsTrait for PageTableFlags {
    fn new() -> Self {
        Self::empty()
    }

    fn set_present(mut self, present: bool) -> Self {
        self.set(Self::V, present);
        self
    }

    fn set_writable(mut self, writable: bool) -> Self {
        self.set(Self::W, writable);
        self
    }

    fn set_readable(self, readable: bool) -> Self {
        self.set(Self::R, readable);
        self
    }

    fn set_accessible_by_user(mut self, accessible: bool) -> Self {
        self.set(Self::U, accessible);
        self
    }

    fn is_present(&self) -> bool {
        self.contains(Self::V)
    }

    fn writable(&self) -> bool {
        self.contains(Self::W)
    }

    fn readable(&self) -> bool {
        self.contains(Self::R)
    }

    fn accessible_by_user(&self) -> bool {
        self.contains(Self::U)
    }

    fn set_executable(mut self, executable: bool) -> Self {
        self.set(Self::X, executable);
        self
    }

    fn executable(&self) -> bool {
        self.contains(Self::X)
    }

    fn has_accessed(&self) -> bool {
        self.contains(Self::A)
    }

    fn is_dirty(&self) -> bool {
        self.contains(Self::D)
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
        self.contains(Self::X) || self.contains(Self::R) || self.contains(Self::W)
    }

    fn set_huge(mut self, huge: bool) -> Self {
        self.set(Self::R, huge);
        self
    }
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct PageTableEntry(usize);

impl PageTableEntry {
    /// 53:10
    const PHYS_ADDR_MASK: usize = 0x1F_FFFF_FFFF_FC00;
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
        va >> (12 + 9 * (level - 1)) & (NR_ENTRIES_PER_PAGE - 1)
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

pub fn tlb_flush() {
    unsafe {
        riscv::asm::sfence_vma_all();
    }
}

pub fn page_table_base() -> Paddr {
    riscv::register::satp::read().ppn() << 12
}

pub const fn is_user_vaddr(vaddr: Vaddr) -> bool {
    // This is Sv48 specific.
    (vaddr >> 47) == 0
}

pub const fn is_kernel_vaddr(vaddr: Vaddr) -> bool {
    // This is Sv48 specific.
    ((vaddr >> 47) & 0x1) == 1
}
