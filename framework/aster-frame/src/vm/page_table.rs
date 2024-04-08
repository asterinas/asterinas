// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};
use core::{fmt::Debug, marker::PhantomData, mem::size_of};

use log::trace;
use pod::Pod;
use spin::Once;

use super::{paddr_to_vaddr, Paddr, Vaddr, VmAllocOptions};
use crate::{
    arch::mm::{is_kernel_vaddr, is_user_vaddr, tlb_flush, PageTableEntry, NR_ENTRIES_PER_PAGE},
    sync::SpinLock,
    vm::{VmFrame, PAGE_SIZE},
};

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

    /// To determine whether the PTE is used, it usually checks whether it is 0.
    ///
    /// The page table will first use this value to determine whether a new page needs to be created to complete the mapping.
    fn is_used(&self) -> bool;

    /// Clear the PTE and reset it to the initial state, which is usually 0.
    fn clear(&mut self);

    /// The index of the next PTE is determined based on the virtual address and the current level, and the level range is [1,5].
    ///
    /// For example, in x86 we use the following expression to get the index (NR_ENTRIES_PER_PAGE is 512):
    /// ```
    /// va >> (12 + 9 * (level - 1)) & (NR_ENTRIES_PER_PAGE - 1)
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
pub enum AddressWidth {
    Level3 = 3,
    Level4 = 4,
    Level5 = 5,
}

#[derive(Debug)]
pub enum PageTableError {
    /// Modifications to page tables (map, unmap, protect, etc.) are invalid for the following reasons:
    ///
    /// 1. The mapping is present before map operation.
    /// 2. The mapping is already invalid before unmap operation.
    /// 3. The mapping is not exists before protect operation.
    InvalidModification,
    InvalidVaddr,
}

pub static KERNEL_PAGE_TABLE: Once<SpinLock<PageTable<PageTableEntry, KernelMode>>> = Once::new();

#[derive(Clone)]
pub struct UserMode {}

#[derive(Clone)]
pub struct KernelMode {}

/// The page table used by iommu maps the device address
/// space to the physical address space.
#[derive(Clone)]
pub struct DeviceMode {}

#[derive(Clone, Debug)]
pub struct PageTable<T: PageTableEntryTrait, M = UserMode> {
    root_paddr: Paddr,
    /// store all the physical frame that the page table need to map all the frame e.g. the frame of the root_pa
    tables: Vec<VmFrame>,
    config: PageTableConfig,
    _phantom: PhantomData<(T, M)>,
}

impl<T: PageTableEntryTrait> PageTable<T, UserMode> {
    pub fn new(config: PageTableConfig) -> Self {
        let root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        Self {
            root_paddr: root_frame.start_paddr(),
            tables: vec![root_frame],
            config,
            _phantom: PhantomData,
        }
    }

    pub fn map(
        &mut self,
        vaddr: Vaddr,
        frame: &VmFrame,
        flags: T::F,
    ) -> Result<(), PageTableError> {
        if is_kernel_vaddr(vaddr) {
            return Err(PageTableError::InvalidVaddr);
        }
        // Safety:
        // 1. The vaddr belongs to user mode program and does not affect the kernel mapping.
        // 2. The area where the physical address islocated at untyped memory and does not affect kernel security.
        unsafe { self.do_map(vaddr, frame.start_paddr(), flags) }
    }

    pub fn unmap(&mut self, vaddr: Vaddr) -> Result<(), PageTableError> {
        if is_kernel_vaddr(vaddr) {
            return Err(PageTableError::InvalidVaddr);
        }
        // Safety: The vaddr belongs to user mode program and does not affect the kernel mapping.
        unsafe { self.do_unmap(vaddr) }
    }

    pub fn protect(&mut self, vaddr: Vaddr, flags: T::F) -> Result<T::F, PageTableError> {
        if is_kernel_vaddr(vaddr) {
            return Err(PageTableError::InvalidVaddr);
        }
        // Safety: The vaddr belongs to user mode program and does not affect the kernel mapping.
        unsafe { self.do_protect(vaddr, flags) }
    }

    /// Add a new mapping directly in the root page table.
    ///
    /// # Safety
    ///
    /// User must guarantee the validity of the PTE.
    pub(crate) unsafe fn add_root_mapping(&mut self, index: usize, pte: &T) {
        debug_assert!((index + 1) * size_of::<T>() <= PAGE_SIZE);
        // Safety: The root_paddr is refer to the root of a valid page table.
        let root_ptes: &mut [T] = table_of(self.root_paddr).unwrap();
        root_ptes[index] = *pte;
    }
}

impl<T: PageTableEntryTrait> PageTable<T, KernelMode> {
    /// Mapping `vaddr` to `paddr` with flags. The `vaddr` should not be at the low address
    ///  (memory belonging to the user mode program).
    ///
    /// # Safety
    ///
    /// Modifying kernel mappings is considered unsafe, and incorrect operation may cause crashes.
    /// User must take care of the consequences when using this API.
    pub unsafe fn map(
        &mut self,
        vaddr: Vaddr,
        paddr: Paddr,
        flags: T::F,
    ) -> Result<(), PageTableError> {
        if is_user_vaddr(vaddr) {
            return Err(PageTableError::InvalidVaddr);
        }
        self.do_map(vaddr, paddr, flags)
    }

    /// Unmap `vaddr`. The `vaddr` should not be at the low address
    ///  (memory belonging to the user mode program).
    ///
    /// # Safety
    ///
    /// Modifying kernel mappings is considered unsafe, and incorrect operation may cause crashes.
    /// User must take care of the consequences when using this API.
    pub unsafe fn unmap(&mut self, vaddr: Vaddr) -> Result<(), PageTableError> {
        if is_user_vaddr(vaddr) {
            return Err(PageTableError::InvalidVaddr);
        }
        self.do_unmap(vaddr)
    }

    /// Modify the flags mapped at `vaddr`. The `vaddr` should not be at the low address
    ///  (memory belonging to the user mode program).
    /// If the modification succeeds, it will return the old flags of `vaddr`.
    ///
    /// # Safety
    ///
    /// Modifying kernel mappings is considered unsafe, and incorrect operation may cause crashes.
    /// User must take care of the consequences when using this API.
    pub unsafe fn protect(&mut self, vaddr: Vaddr, flags: T::F) -> Result<T::F, PageTableError> {
        if is_user_vaddr(vaddr) {
            return Err(PageTableError::InvalidVaddr);
        }
        self.do_protect(vaddr, flags)
    }
}

impl<T: PageTableEntryTrait> PageTable<T, DeviceMode> {
    pub fn new(config: PageTableConfig) -> Self {
        let root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        Self {
            root_paddr: root_frame.start_paddr(),
            tables: vec![root_frame],
            config,
            _phantom: PhantomData,
        }
    }

    /// Mapping directly from a virtual address to a physical address.
    /// The virtual address should be in the device address space.
    ///
    /// # Safety
    ///
    /// User must ensure the given paddr is a valid one (e.g. from the VmSegment).
    pub unsafe fn map_with_paddr(
        &mut self,
        vaddr: Vaddr,
        paddr: Paddr,
        flags: T::F,
    ) -> Result<(), PageTableError> {
        self.do_map(vaddr, paddr, flags)
    }

    pub fn unmap(&mut self, vaddr: Vaddr) -> Result<(), PageTableError> {
        // Safety: the `vaddr` is in the device address space.
        unsafe { self.do_unmap(vaddr) }
    }
}

impl<T: PageTableEntryTrait, M> PageTable<T, M> {
    /// Mapping `vaddr` to `paddr` with flags.
    ///
    /// # Safety
    ///
    /// This function allows arbitrary modifications to the page table.
    /// Incorrect modifications may cause the kernel to crash (e.g., changing the linear mapping.).
    unsafe fn do_map(
        &mut self,
        vaddr: Vaddr,
        paddr: Paddr,
        flags: T::F,
    ) -> Result<(), PageTableError> {
        let last_entry = self.do_page_walk_mut(vaddr, true).unwrap();
        trace!(
            "Page Table: Map vaddr:{:x?}, paddr:{:x?}, flags:{:x?}",
            vaddr,
            paddr,
            flags
        );
        if last_entry.is_used() && last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.update(paddr, flags);
        tlb_flush(vaddr);
        Ok(())
    }

    /// Find the last PTE and return its mutable reference.
    ///
    /// If create is set, it will create the next table until the last PTE.
    /// If not, it will return `None` if it cannot reach the last PTE.
    fn do_page_walk_mut(&mut self, vaddr: Vaddr, create: bool) -> Option<&mut T> {
        let mut level = self.config.address_width as usize;
        // Safety: The offset does not exceed the value of PAGE_SIZE.
        // It only change the memory controlled by page table.
        let mut current: &mut T =
            unsafe { &mut *(calculate_pte_vaddr::<T>(self.root_paddr, vaddr, level) as *mut T) };

        while level > 1 {
            if !current.flags().is_present() {
                if !create {
                    return None;
                }
                // Create next table
                let frame = VmAllocOptions::new(1).alloc_single().unwrap();
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
            level -= 1;
            // Safety: The offset does not exceed the value of PAGE_SIZE.
            // It only change the memory controlled by page table.
            current = unsafe {
                &mut *(calculate_pte_vaddr::<T>(current.paddr(), vaddr, level) as *mut T)
            };
        }
        Some(current)
    }

    /// Find the last PTE and return its immutable reference.
    ///
    /// This function will return `None` if it cannot reach the last PTE.
    /// Note that finding an entry does not mean the corresponding virtual memory address is mapped
    /// since the entry may be empty.
    fn do_page_walk(&self, vaddr: Vaddr) -> Option<&T> {
        let mut level = self.config.address_width as usize;
        // Safety: The offset does not exceed the value of PAGE_SIZE.
        // It only change the memory controlled by page table.
        let mut current: &T =
            unsafe { &*(calculate_pte_vaddr::<T>(self.root_paddr, vaddr, level) as *const T) };

        while level > 1 {
            if !current.flags().is_present() {
                return None;
            }
            if current.flags().is_huge() {
                break;
            }
            level -= 1;
            // Safety: The offset does not exceed the value of PAGE_SIZE.
            // It only change the memory controlled by page table.
            current =
                unsafe { &*(calculate_pte_vaddr::<T>(current.paddr(), vaddr, level) as *const T) };
        }
        Some(current)
    }

    /// Unmap `vaddr`.
    ///
    /// # Safety
    ///
    /// This function allows arbitrary modifications to the page table.
    /// Incorrect modifications may cause the kernel to crash (e.g., unmap the linear mapping.).
    unsafe fn do_unmap(&mut self, vaddr: Vaddr) -> Result<(), PageTableError> {
        let last_entry = self
            .do_page_walk_mut(vaddr, false)
            .ok_or(PageTableError::InvalidModification)?;
        trace!("Page Table: Unmap vaddr:{:x?}", vaddr);
        if !last_entry.is_used() || !last_entry.flags().is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.clear();
        tlb_flush(vaddr);
        Ok(())
    }

    /// Modify the flags mapped at `vaddr`.
    /// If the modification succeeds, it will return the old flags of `vaddr`.
    ///
    /// # Safety
    ///
    /// This function allows arbitrary modifications to the page table.
    /// Incorrect modifications may cause the kernel to crash
    /// (e.g., make the linear mapping visible to the user mode applications.).
    unsafe fn do_protect(&mut self, vaddr: Vaddr, new_flags: T::F) -> Result<T::F, PageTableError> {
        let last_entry = self
            .do_page_walk_mut(vaddr, false)
            .ok_or(PageTableError::InvalidModification)?;
        let old_flags = last_entry.flags();
        trace!(
            "Page Table: Protect vaddr:{:x?}, flags:{:x?}",
            vaddr,
            new_flags
        );
        if !last_entry.is_used() || !old_flags.is_present() {
            return Err(PageTableError::InvalidModification);
        }
        last_entry.update(last_entry.paddr(), new_flags);
        tlb_flush(vaddr);
        Ok(old_flags)
    }

    /// Construct a page table instance from root registers (CR3 in x86)
    ///
    /// # Safety
    ///
    /// This function bypasses Rust's ownership model and directly constructs an instance of a
    /// page table.
    pub(crate) unsafe fn from_root_register() -> Self {
        #[cfg(target_arch = "x86_64")]
        let (page_directory_base, _) = x86_64::registers::control::Cr3::read();
        PageTable {
            root_paddr: page_directory_base.start_address().as_u64() as usize,
            tables: Vec::new(),
            config: PageTableConfig {
                address_width: AddressWidth::Level4,
            },
            _phantom: PhantomData,
        }
    }

    /// Return the flags of the PTE for the target virtual memory address.
    /// If the PTE does not exist, return `None`.
    pub fn flags(&self, vaddr: Vaddr) -> Option<T::F> {
        self.do_page_walk(vaddr).map(|entry| entry.flags())
    }

    /// Return the root physical address of current `PageTable`.
    pub fn root_paddr(&self) -> Paddr {
        self.root_paddr
    }

    /// Determine whether the target virtual memory address is mapped.
    pub fn is_mapped(&self, vaddr: Vaddr) -> bool {
        self.do_page_walk(vaddr)
            .is_some_and(|last_entry| last_entry.is_used() && last_entry.flags().is_present())
    }
}

/// Read `NR_ENTRIES_PER_PAGE` of PageTableEntry from an address
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
    Some(core::slice::from_raw_parts_mut(ptr, NR_ENTRIES_PER_PAGE))
}

/// translate a virtual address to physical address which cannot use offset to get physical address
pub fn vaddr_to_paddr(vaddr: Vaddr) -> Option<Paddr> {
    let page_table = KERNEL_PAGE_TABLE.get().unwrap().lock();
    // Although we bypass the unsafe APIs provided by KernelMode, the purpose here is
    // only to obtain the corresponding physical address according to the mapping.
    let last_entry = page_table.do_page_walk(vaddr)?;
    // FIXME: Support huge page
    Some(last_entry.paddr() + (vaddr & (PAGE_SIZE - 1)))
}

fn calculate_pte_vaddr<T: PageTableEntryTrait>(
    root_pa: Paddr,
    target_va: Vaddr,
    level: usize,
) -> Vaddr {
    debug_assert!(size_of::<T>() * (T::page_index(target_va, level) + 1) <= PAGE_SIZE);
    paddr_to_vaddr(root_pa + size_of::<T>() * T::page_index(target_va, level))
}

pub fn init() {
    KERNEL_PAGE_TABLE.call_once(|| {
        // Safety: The `KERENL_PAGE_TABLE` is the only page table that is used to modify the initialize
        // mapping.
        SpinLock::new(unsafe { PageTable::from_root_register() })
    });
}
