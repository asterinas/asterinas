use super::{
    frame::VmFrameFlags,
    frame_allocator, paddr_to_vaddr, VmAllocOptions, VmFrameVec, {Paddr, Vaddr},
};
use crate::{
    arch::mm::{tlb_flush, PageTableEntry},
    config::{ENTRY_COUNT, PAGE_SIZE},
    vm::VmFrame,
};
use alloc::{vec, vec::Vec};
use core::{fmt::Debug, marker::PhantomData, mem::size_of};
use log::trace;
use pod::Pod;

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

    fn new(paddr: Paddr, flags: Self::F) -> Self;

    fn paddr(&self) -> Paddr;

    fn flags(&self) -> Self::F;

    fn update(&mut self, paddr: Paddr, flags: Self::F);

    /// To determine whether the PTE is unused, it usually checks whether it is 0.
    ///
    /// The page table will first use this value to determine whether a new page needs to be created to complete the mapping.
    fn is_unused(&self) -> bool;

    /// Clear the PTE and reset it to the initial state, which is usually 0.
    fn clear(&mut self);

    /// The index of the next PTE is determined based on the virtual address and the current level, and the level range is [1,5].
    ///
    /// For example, in x86 we use the following expression to get the index (ENTRY_COUNT is 512):
    /// ```
    /// va >> (12 + 9 * (level - 1)) & (ENTRY_COUNT - 1)
    /// ```
    ///
    fn page_index(va: Vaddr, level: usize) -> usize;
}

#[derive(Debug, Clone, Copy)]
pub struct PageTableConfig {
    pub address_width: AddressWidth,
}

#[derive(Debug, Clone, Copy)]
#[repr(usize)]
#[allow(clippy::enum_variant_names)]
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
        let last_entry = self.page_walk(vaddr, true).unwrap();
        trace!(
            "Page Table: Map vaddr:{:x?}, paddr:{:x?}, flags:{:x?}",
            vaddr,
            paddr,
            flags
        );
        if !last_entry.is_unused() && last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.update(paddr, flags);
        tlb_flush(vaddr);
        Ok(())
    }

    /// Find the last PTE
    ///
    /// If create is set, it will create the next table until the last PTE.
    /// If not, it will return None if it is not reach the last PTE.
    ///
    fn page_walk(&mut self, vaddr: Vaddr, create: bool) -> Option<&mut T> {
        let mut count = self.config.address_width as usize;
        debug_assert!(size_of::<T>() * (T::page_index(vaddr, count) + 1) <= PAGE_SIZE);
        // Safety: The offset does not exceed the value of PAGE_SIZE.
        // It only change the memory controlled by page table.
        let mut current: &mut T = unsafe {
            &mut *(paddr_to_vaddr(self.root_pa + size_of::<T>() * T::page_index(vaddr, count))
                as *mut T)
        };

        while count > 1 {
            if !current.flags().is_present() {
                if !create {
                    return None;
                }
                // Create next table
                let frame = VmFrameVec::allocate(VmAllocOptions::new(1).uninit(false))
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
            if current.flags().is_huge() {
                break;
            }
            count -= 1;
            debug_assert!(size_of::<T>() * (T::page_index(vaddr, count) + 1) <= PAGE_SIZE);
            // Safety: The offset does not exceed the value of PAGE_SIZE.
            // It only change the memory controlled by page table.
            current = unsafe {
                &mut *(paddr_to_vaddr(
                    current.paddr() + size_of::<T>() * T::page_index(vaddr, count),
                ) as *mut T)
            };
        }
        Some(current)
    }

    pub fn unmap(&mut self, vaddr: Vaddr) -> Result<(), PageTableError> {
        let last_entry = self.page_walk(vaddr, false).unwrap();
        trace!("Page Table: Unmap vaddr:{:x?}", vaddr);
        if last_entry.is_unused() && !last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.clear();
        tlb_flush(vaddr);
        Ok(())
    }

    pub fn protect(&mut self, vaddr: Vaddr, flags: T::F) -> Result<(), PageTableError> {
        let last_entry = self.page_walk(vaddr, false).unwrap();
        trace!("Page Table: Protect vaddr:{:x?}, flags:{:x?}", vaddr, flags);
        if last_entry.is_unused() || !last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.update(last_entry.paddr(), flags);
        tlb_flush(vaddr);
        Ok(())
    }

    pub fn flags(&mut self, vaddr: Vaddr) -> Option<T::F> {
        let last_entry = self.page_walk(vaddr, false)?;
        Some(last_entry.flags())
    }

    pub fn root_paddr(&self) -> Paddr {
        self.root_pa
    }
}

/// get page table from page table base register
pub fn current_page_table() -> PageTable<PageTableEntry> {
    #[cfg(target_arch = "x86_64")]
    let (page_directory_base, _) = x86_64::registers::control::Cr3::read();

    // TODO: Read and use different level page table.
    // Safety: The page_directory_base is valid since it is read from PDBR.
    // We only use this instance to do the page walk without creating.
    let page_table: PageTable<PageTableEntry> = unsafe {
        PageTable::from_paddr(
            PageTableConfig {
                address_width: AddressWidth::Level4PageTable,
            },
            page_directory_base.start_address().as_u64() as usize,
        )
    };
    page_table
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

/// translate a virtual address to physical address which cannot use offset to get physical address
pub fn vaddr_to_paddr(vaddr: Vaddr) -> Option<Paddr> {
    let mut page_table = current_page_table();
    let last_entry = page_table.page_walk(vaddr, false)?;
    // FIXME: Support huge page
    Some(last_entry.paddr() + (vaddr & (PAGE_SIZE - 1)))
}
