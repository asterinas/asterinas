use super::{
    frame::VmFrameFlags,
    frame_allocator, paddr_to_vaddr, VmAllocOptions, VmFrameVec, {Paddr, Vaddr},
};
use crate::{
    config::{self, ENTRY_COUNT, PAGE_SIZE},
    vm::VmFrame,
};
use alloc::{vec, vec::Vec};
use bit_field::BitField;
use core::{fmt::Debug, marker::PhantomData, mem::size_of};
use log::trace;
use pod::Pod;

#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord)]
#[repr(u16)]
pub enum PageSize {
    Size4KiB = 1,
    Size2MiB = 2,
    Size1GiB = 3,
}

impl PageSize {
    /// Get the final physical address through PTE and virtual address.
    pub fn final_paddr<T: PageTableEntryTrait>(
        &self,
        pte: &T,
        vaddr: Vaddr,
        address_width: AddressWidth,
    ) -> Paddr {
        let page_bits = (T::VADDR_OFFSET_BITS + T::VADDR_INDEX_BITS * (*self as u16 - 1)) as usize;
        let max_bits = (T::VADDR_OFFSET_BITS + address_width as u16 * T::VADDR_INDEX_BITS) as usize;
        let phys_frame = pte.raw().get_bits(page_bits..max_bits) << page_bits;
        vaddr.get_bits(0..page_bits) | phys_frame
    }
}

pub trait PageTableFlagsTrait: Clone + Copy + Sized + Pod + Debug {
    fn new() -> Self;

    fn set_present(self, present: bool) -> Self;

    fn set_writable(self, writable: bool) -> Self;

    fn set_readable(self, readable: bool) -> Self;

    fn set_accessible_by_user(self, accessible: bool) -> Self;

    fn set_executable(self, executable: bool) -> Self;

    fn set_huge(self, huge: bool) -> Self;

    fn is_present(&self) -> bool;

    fn writable(&self) -> bool;

    fn readable(&self) -> bool;

    fn executable(&self) -> bool;

    fn has_accessed(&self) -> bool;

    fn is_dirty(&self) -> bool;

    fn is_huge(&self) -> bool;

    fn accessible_by_user(&self) -> bool;

    /// Returns a new set of flags, containing any flags present in either self or other. It is similar to the OR operation.
    fn union(&self, other: &Self) -> Self;

    /// Remove the specified flags.
    fn remove(&mut self, flags: &Self);

    /// Insert the specified flags.
    fn insert(&mut self, flags: &Self);
}

pub trait PageTableEntryTrait: Clone + Copy + Sized + Pod + Debug {
    type F: PageTableFlagsTrait;
    /// The index bits of the vaddr. For example, in x86-64 the index is 9.
    const VADDR_INDEX_BITS: u16;
    /// The default offset bits of the vaddr. For example, in x86-64 the default offset is 12(4KiB).
    const VADDR_OFFSET_BITS: u16;

    fn new(paddr: Paddr, flags: Self::F) -> Self;

    fn paddr(&self) -> Paddr;

    fn flags(&self) -> Self::F;

    fn raw(&self) -> usize;

    fn update(&mut self, paddr: Paddr, flags: Self::F);

    /// To determine whether the PTE is unused, it usually checks whether it is 0.
    ///
    /// The page table will first use this value to determine whether a new page needs to be created to complete the mapping.
    fn is_unused(&self) -> bool;

    /// Clear the PTE and reset it to the initial state, which is usually 0.
    fn clear(&mut self);
}

#[derive(Debug, Clone, Copy)]
pub struct PageTableConfig {
    pub address_width: AddressWidth,
}

#[derive(Debug, Clone, Copy)]
#[repr(u16)]
pub enum AddressWidth {
    Level3PageTable = 3,
    Level4PageTable = 4,
    Level5PageTable = 5,
}

#[derive(Debug)]
pub enum PageTableError {
    /// Modifications to page tables (map, unmap, protect, etc.) are invalid for the following reasons:
    ///
    /// 1. The mapping is present before map operation.
    /// 2. The mapping is already invalid before unmap operation.
    /// 3. The mapping is not exists before protect operation.
    InvalidModification,
}

#[derive(Clone, Debug)]
pub struct PageTable<T: PageTableEntryTrait> {
    pub root_pa: Paddr,
    /// store all the physical frame that the page table need to map all the frame e.g. the frame of the root_pa
    tables: Vec<VmFrame>,
    config: PageTableConfig,
    _phantom: PhantomData<T>,
}

impl<T: PageTableEntryTrait> PageTable<T> {
    pub fn new(config: PageTableConfig) -> Self {
        let root_frame = frame_allocator::alloc_zero(VmFrameFlags::empty()).unwrap();
        Self {
            root_pa: root_frame.start_paddr(),
            tables: vec![root_frame],
            config,
            _phantom: PhantomData,
        }
    }

    /// Create the page table structure according to the physical address, note that the created page table can only use the page_walk function without create.
    ///
    /// # Safety
    ///
    /// User should ensure the physical address is valid and only invoke the `page_walk` function without creating new PTE.
    ///
    pub unsafe fn from_paddr(config: PageTableConfig, paddr: Paddr) -> Self {
        Self {
            root_pa: paddr,
            tables: Vec::new(),
            config,
            _phantom: PhantomData,
        }
    }

    /// Add a new mapping directly in the root page table.
    ///
    /// # Safety
    ///
    /// User must guarantee the validity of the PTE.
    ///
    pub unsafe fn add_root_mapping(&mut self, index: usize, pte: &T) {
        debug_assert!((index + 1) * size_of::<T>() <= PAGE_SIZE);
        // Safety: The root_pa is refer to the root of a valid page table.
        let root_ptes: &mut [T] = table_of(self.root_pa).unwrap();
        root_ptes[index] = *pte;
    }

    pub fn map(&mut self, vaddr: Vaddr, paddr: Paddr, flags: T::F) -> Result<(), PageTableError> {
        let (last_entry, page_size) = self.page_walk(vaddr, true).unwrap();
        trace!(
            "Page Table: Map vaddr:{:x?}, paddr:{:x?}, flags:{:x?}",
            vaddr,
            paddr,
            flags
        );
        debug_assert!(vaddr < config::PHYS_OFFSET);
        debug_assert!(page_size == PageSize::Size4KiB);
        if !last_entry.is_unused() && last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.update(paddr, flags);
        Ok(())
    }

    /// Find the last PTE
    ///
    /// If create is set, it will create the next table until the last PTE.
    /// If not, it will return None if it is not reach the last PTE.
    ///
    pub(crate) fn page_walk(&mut self, vaddr: Vaddr, create: bool) -> Option<(&mut T, PageSize)> {
        let mut level = self.config.address_width as u16;
        let page_index = vaddr >> (T::VADDR_OFFSET_BITS + T::VADDR_INDEX_BITS * (level - 1))
            & ((1 << T::VADDR_INDEX_BITS) - 1);
        debug_assert!(size_of::<T>() * (page_index + 1) <= PAGE_SIZE);
        // Safety: The offset does not exceed the value of PAGE_SIZE.
        // It only change the memory controlled by page table.
        let mut current: &mut T =
            unsafe { &mut *(paddr_to_vaddr(self.root_pa + size_of::<T>() * page_index) as *mut T) };

        while level > 1 && !current.flags().is_huge() {
            if !current.flags().is_present() {
                if !create {
                    return None;
                }
                // Create next table
                let frame = VmFrameVec::allocate(&VmAllocOptions::new(1).uninit(false))
                    .unwrap()
                    .pop()
                    .unwrap();
                // Default flags: read, write, user, present
                let flags = T::F::new()
                    .set_present(true)
                    .set_accessible_by_user(true)
                    .set_readable(true)
                    .set_writable(true);
                current.update(frame.start_paddr(), flags);
                self.tables.push(frame);
            }
            level -= 1;
            let page_index = vaddr >> (T::VADDR_OFFSET_BITS + T::VADDR_INDEX_BITS * (level - 1))
                & ((1 << T::VADDR_INDEX_BITS) - 1);
            debug_assert!(size_of::<T>() * (page_index + 1) <= PAGE_SIZE);
            // Safety: The offset does not exceed the value of PAGE_SIZE.
            // It only change the memory controlled by page table.
            current = unsafe {
                &mut *(paddr_to_vaddr(current.paddr() + size_of::<T>() * page_index) as *mut T)
            };
        }
        let page_size = match level {
            1 => PageSize::Size4KiB,
            2 => PageSize::Size2MiB,
            3 => PageSize::Size1GiB,
            _ => unreachable!(),
        };
        Some((current, page_size))
    }

    pub fn unmap(&mut self, vaddr: Vaddr) -> Result<(), PageTableError> {
        let (last_entry, page_size) = self.page_walk(vaddr, false).unwrap();
        trace!("Page Table: Unmap vaddr:{:x?}", vaddr);
        debug_assert!(vaddr < config::PHYS_OFFSET);
        debug_assert!(page_size == PageSize::Size4KiB);
        if last_entry.is_unused() && !last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.clear();
        Ok(())
    }

    pub fn protect(&mut self, vaddr: Vaddr, flags: T::F) -> Result<(), PageTableError> {
        let (last_entry, page_size) = self.page_walk(vaddr, false).unwrap();
        trace!("Page Table: Protect vaddr:{:x?}, flags:{:x?}", vaddr, flags);
        debug_assert!(vaddr < config::PHYS_OFFSET);
        debug_assert!(page_size == PageSize::Size4KiB);
        if last_entry.is_unused() || !last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.update(last_entry.paddr(), flags);
        Ok(())
    }

    pub fn root_paddr(&self) -> Paddr {
        self.root_pa
    }
}

/// Read `ENTRY_COUNT` of PageTableEntry from an address
///
/// # Safety
///
/// User must ensure that the physical address refers to the root of a valid page table.
///
pub unsafe fn table_of<'a, T: PageTableEntryTrait>(pa: Paddr) -> Option<&'a mut [T]> {
    if pa == 0 {
        return None;
    }
    let ptr = super::paddr_to_vaddr(pa) as *mut _;
    Some(core::slice::from_raw_parts_mut(ptr, ENTRY_COUNT))
}
