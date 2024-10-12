// SPDX-License-Identifier: MPL-2.0

//! This module provides abstraction over the Intel IA32E paging mechanism. And
//! offers method to create linear page tables.
//!
//! Notebly, the 4-level page table has a paging structure named as follows:
//!  - Level-4: Page Map Level 4 (PML4), or "the root page table";
//!  - Level-3: Page Directory Pointer Table (PDPT);
//!  - Level-2: Page Directory (PD);
//!  - Level-1: Page Table (PT).
//!
//! We sometimes use "level-n" page table to refer to the page table described
//! above, avoiding the use of complicated names in the Intel manual.

use x86_64::structures::paging::PhysFrame;

const TABLE_ENTRY_COUNT: usize = 512;

bitflags::bitflags! {
    #[derive(Clone, Copy)]
    #[repr(C)]
    pub struct Ia32eFlags: u64 {
        const PRESENT =         1 << 0;
        const WRITABLE =        1 << 1;
        const USER =            1 << 2;
        const WRITE_THROUGH =   1 << 3;
        const NO_CACHE =        1 << 4;
        const ACCESSED =        1 << 5;
        const DIRTY =           1 << 6;
        const HUGE =            1 << 7;
        const GLOBAL =          1 << 8;
        const NO_EXECUTE =      1 << 63;
    }
}

#[repr(C)]
pub struct Ia32eEntry(u64);

/// The table in the IA32E paging specification that occupies a physical page frame.
#[repr(C)]
pub struct Ia32eTable([Ia32eEntry; TABLE_ENTRY_COUNT]);

/// A page number. It could be either a physical page number or a virtual page number.
#[derive(Copy, Clone, Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct PageNumber(u64);

fn is_4k_page_aligned(addr: u64) -> bool {
    addr & 0xfff == 0
}

impl PageNumber {
    /// Creates a new page number from the given address.
    pub fn from_addr(addr: u64) -> Self {
        assert!(is_4k_page_aligned(addr));
        Self(addr >> 12)
    }
    /// Returns the address of the page.
    pub fn addr(&self) -> u64 {
        self.0 << 12
    }
    /// Get the physical page frame as slice.
    ///
    /// # Safety
    /// The caller must ensure that the page number is a physical page number and
    /// it is identically mapped when running the code.
    unsafe fn get_page_frame(&self) -> &'static mut [u8] {
        core::slice::from_raw_parts_mut(self.addr() as *mut u8, 4096)
    }
}

impl core::ops::Add<usize> for PageNumber {
    type Output = Self;
    fn add(self, rhs: usize) -> Self::Output {
        Self(self.0 + rhs as u64)
    }
}

impl core::ops::AddAssign<usize> for PageNumber {
    fn add_assign(&mut self, rhs: usize) {
        self.0 += rhs as u64;
    }
}

impl core::ops::Sub<PageNumber> for PageNumber {
    type Output = u64;
    fn sub(self, rhs: PageNumber) -> Self::Output {
        self.0 - rhs.0
    }
}

/// A creator for a page table.
///
/// It allocates page frames from the given physical memory range. And the first
/// page frame is always used for the PML4 table (root page table).
pub struct PageTableCreator {
    first_pfn: PageNumber,
    next_pfn: PageNumber,
    end_pfn: PageNumber,
}

/// Fills the given slice with the given value.
///
/// TODO: use `Slice::fill` instead. But it currently will fail with "invalid opcode".
unsafe fn memset(dst: &mut [u8], val: u8) {
    core::arch::asm!(
        "rep stosb",
        inout("rcx") dst.len() => _,
        inout("rdi") dst.as_mut_ptr() => _,
        in("al") val,
        options(nostack),
    );
}

impl PageTableCreator {
    /// Creates a new page table creator.
    ///
    /// The input physical memory range must be at least 4 page frames. New
    /// mappings will be written into the given physical memory range.
    ///
    /// # Safety
    /// The caller must ensure that the given physical memory range is valid.
    pub unsafe fn new(first_pfn: PageNumber, end_pfn: PageNumber) -> Self {
        assert!(end_pfn - first_pfn >= 4);
        // Clear the first page for the PML4 table.
        memset(first_pfn.get_page_frame(), 0);
        Self {
            first_pfn,
            next_pfn: first_pfn + 1,
            end_pfn,
        }
    }

    fn allocate(&mut self) -> PageNumber {
        assert!(self.next_pfn < self.end_pfn);
        let pfn = self.next_pfn;
        self.next_pfn += 1;
        unsafe {
            memset(pfn.get_page_frame(), 0);
        }
        pfn
    }

    pub fn map(&mut self, from: PageNumber, to: PageNumber, flags: Ia32eFlags) {
        let pml4 = unsafe { &mut *(self.first_pfn.addr() as *mut Ia32eTable) };
        let pml4e = pml4.index(4, from.addr());
        if !pml4e.flags().contains(Ia32eFlags::PRESENT) {
            let pdpt_pfn = self.allocate();
            pml4e.update(pdpt_pfn.addr(), flags);
        }
        let pdpt = unsafe { &mut *(pml4e.paddr() as *mut Ia32eTable) };
        let pdpte = pdpt.index(3, from.addr());
        if !pdpte.flags().contains(Ia32eFlags::PRESENT) {
            let pd_pfn = self.allocate();
            pdpte.update(pd_pfn.addr(), flags);
        }
        let pd = unsafe { &mut *(pdpte.paddr() as *mut Ia32eTable) };
        let pde = pd.index(2, from.addr());
        if !pde.flags().contains(Ia32eFlags::PRESENT) {
            let pt_pfn = self.allocate();
            pde.update(pt_pfn.addr(), flags);
        }
        let pt = unsafe { &mut *(pde.paddr() as *mut Ia32eTable) };
        let pte = pt.index(1, from.addr());
        // In level-1 PTE, the HUGE bit is the PAT bit (page attribute table).
        // We use it as the "valid" bit for the page table entry.
        pte.update(to.addr(), flags | Ia32eFlags::HUGE);
    }

    pub fn nr_frames_used(&self) -> usize {
        (self.next_pfn - self.first_pfn).try_into().unwrap()
    }

    /// Activates the created page table.
    ///
    /// # Safety
    /// The caller must ensure that the page table is valid.
    pub unsafe fn activate(&self, flags: x86_64::registers::control::Cr3Flags) {
        x86_64::registers::control::Cr3::write(
            PhysFrame::from_start_address(x86_64::PhysAddr::new(self.first_pfn.addr())).unwrap(),
            flags,
        );
    }
}

impl Ia32eTable {
    fn index(&mut self, level: usize, va: u64) -> &mut Ia32eEntry {
        debug_assert!((1..=5).contains(&level));
        let index = va as usize >> (12 + 9 * (level - 1)) & (TABLE_ENTRY_COUNT - 1);
        &mut self.0[index]
    }
}

impl Ia32eEntry {
    /// 51:12
    const PHYS_ADDR_MASK: u64 = 0xF_FFFF_FFFF_F000;

    fn paddr(&self) -> u64 {
        self.0 & Self::PHYS_ADDR_MASK
    }
    fn flags(&self) -> Ia32eFlags {
        Ia32eFlags::from_bits_truncate(self.0)
    }
    fn update(&mut self, paddr: u64, flags: Ia32eFlags) {
        self.0 = (paddr & Self::PHYS_ADDR_MASK) | flags.bits();
    }
}
