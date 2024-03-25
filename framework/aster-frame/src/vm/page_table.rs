// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};
use num::ToPrimitive;
use core::{fmt::Debug, marker::PhantomData, mem::size_of, ops::Range};

use num_derive::ToPrimitive;
use spin::Once;
use pod::Pod;

use super::{paddr_to_vaddr, Paddr, Vaddr, VmAllocOptions};
use crate::{
    arch::mm::{tlb_flush, PageTableEntry, NR_ENTRIES_PER_PAGE},
    sync::SpinLock,
    vm::{VmFrame, BASE_PAGE_SIZE},
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
    /// The minimum depth of translation.
    /// 
    /// It's value is the level that the page table can start to use huge pages.
    pub supported_translation_depth: usize,
}

#[derive(Debug, Clone, Copy, ToPrimitive)]
#[repr(usize)]
pub enum AddressWidth {
    Level3 = 3,
    Level4 = 4,
    Level5 = 5,
}

impl AddressWidth {
    pub fn page_size_at_level(&self, level: usize) -> usize {
        BASE_PAGE_SIZE << (9 * (self.to_usize().unwrap() - level))
    }
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

/// This is a compile-time technique to force the frame developers to distinguish
/// between the kernel global page table instance, process specific user page table
/// instance, and device page table instances.
pub(crate) trait PageTableMode {
    /// The range of virtual addresses that the page table can manage.
    const VADDR_RANGE: Range<Vaddr>;
}

#[derive(Clone)]
pub struct UserMode {}

impl PageTableMode for UserMode {
    const VADDR_RANGE: Range<Vaddr> = 0..super::MAX_USERSPACE_VADDR;
}

#[derive(Clone)]
pub struct KernelMode {}

impl PageTableMode for KernelMode {
    const VADDR_RANGE: Range<Vaddr> = super::KERNEL_BASE_VADDR..super::KERNEL_END_VADDR;
}

#[derive(Clone, Debug)]
pub struct PageTable<T: PageTableEntryTrait, M = UserMode> {
    root_paddr: Paddr,
    /// store all the physical frame that the page table need to map all the frame e.g. the frame of the root_pa
    tables: Vec<VmFrame>,
    config: PageTableConfig,
    _phantom: PhantomData<(T, M)>,
}

impl<T: PageTableEntryTrait> PageTable<T, UserMode> {
    pub fn map(
        &mut self,
        vaddr: Vaddr,
        frame: &VmFrame,
        flags: T::F,
    ) -> Result<(), PageTableError> {
        if !UserMode::VADDR_RANGE.contains(&vaddr) {
            return Err(PageTableError::InvalidVaddr);
        }
        let from = vaddr..vaddr + BASE_PAGE_SIZE;
        let to = frame.start_paddr()..frame.start_paddr() + BASE_PAGE_SIZE;
        // Safety:
        // 1. The vaddr belongs to user mode program and does not affect the kernel mapping.
        // 2. The area where the physical address islocated at untyped memory and does not affect kernel security.
        unsafe { self.do_map(from, to, MapProperty::from(flags)) }
    }
}

impl<T: PageTableEntryTrait> PageTable<T, KernelMode> {
    pub fn fork(&self) -> PageTable<T, UserMode> {
        let root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        // Safety: The root_paddr is refer to the root of a valid page table.
        unsafe {
            let src = self.root_paddr as *const T;
            let dst = root_frame.start_paddr() as *mut T;
            core::ptr::copy_nonoverlapping(src, dst, NR_ENTRIES_PER_PAGE);
        }
        PageTable::<T, UserMode> {
            root_paddr: root_frame.start_paddr(),
            tables: vec![root_frame],
            config: self.config,
            _phantom: PhantomData,
        }
    }
    pub unsafe fn map(
        from: Range<Vaddr>,
        to: Range<Paddr>,
        op: impl Fn(MapProperty) -> MapProperty
    ) -> Result<(), PageTableError> {
        if !range_contains(KernelMode::VADDR_RANGE, from){
            return Err(PageTableError::InvalidVaddr);
        }
        self.do_map(from, to, op)
    }
}

impl<T: PageTableEntryTrait, M> PageTable<T, M> {
    /// Mapping `vaddr` to `paddr` with flags.
    /// 
    /// This function can be used to map, unmap or protect virtual memory ranges.
    /// The `op` function is for you to customize the operation on the properties
    /// of the range. The argument `F` is the properties on the page table entry
    /// before the mapping operation, and the return value is the properties you
    /// want after the mapping operation.
    /// 
    /// If the argument `to` is `None`, this function will not change the mapped
    /// address but just adjust the flags according to `op`.
    /// 
    /// The function will map as more huge pages as possible, and it will split
    /// the huge pages into smaller pages if necessary. If the input range is large,
    /// the resulting mappings may look like this (if very huge pages supported):
    /// 
    /// ```text
    /// start                                                             end
    ///   |----|----------------|--------------------------------|----|----|
    ///   small      huge                     very huge          small small
    /// on x86:
    ///    4KiB      2MiB                       1GiB             4KiB  4KiB
    /// ```
    pub unsafe fn do_map(
        &mut self,
        from: Range<Vaddr>,
        to: Option<Range<Paddr>>,
        op: impl Fn(MapProperty) -> MapProperty
    ) -> Result<(), PageTableError>
    {   
        let page_sizes = {
            let mut page_sizes = [0; 5];
            for i in 0..5 {
                page_sizes[i] = self.config.address_width.page_size_at_level(i + 1);
            }
            page_sizes
        };
        let cur_level = self.config.address_width.to_usize().unwrap();
        let mut current_pt: &mut T = &mut *(paddr_to_vaddr(self.root_paddr) as *mut T);
        let mut current_vaddr = from.start;

        while cur_level > 1 {
            debug_assert!(size_of::<T>() * (T::page_index(current_vaddr, cur_level) + 1) <= BASE_PAGE_SIZE);
            // Safety: The offset does not exceed the value of BASE_PAGE_SIZE.
            // It only change the memory controlled by page table.
            current_pt = unsafe {
                &mut *(paddr_to_vaddr(
                    current_pt.paddr() + size_of::<T>() * T::page_index(current_vaddr, cur_level),
                ) as *mut T)
            };
        }

        Ok(())
    }

    /// Query the page table for the mapping information of the specified range.
    /// 
    /// The function will return a iterator on a list mappings
    pub fn do_query<'a>(
        &'a self,
        on: Range<Vaddr>,
    ) -> PageTableQueryIter<'a, T, M>
    {
        todo!()
    }

    /// A software emulation of the address translation process.
    pub fn translate(&self, vaddr: Vaddr) -> Option<Paddr> {
        let mut count = self.config.address_width.to_usize().unwrap();
        debug_assert!(size_of::<T>() * (T::page_index(vaddr, count) + 1) <= BASE_PAGE_SIZE);
        // Safety: The offset does not exceed the value of BASE_PAGE_SIZE.
        // It only change the memory controlled by page table.
        let mut current: &mut T = unsafe {
            &mut *(paddr_to_vaddr(self.root_paddr + size_of::<T>() * T::page_index(vaddr, count))
                as *mut T)
        };

        while count > 1 {
            if current.flags().is_huge() {
                debug_assert!(count <= self.config.supported_translation_depth);
                break;
            }
            count -= 1;
            debug_assert!(size_of::<T>() * (T::page_index(vaddr, count) + 1) <= BASE_PAGE_SIZE);
            // Safety: The offset does not exceed the value of BASE_PAGE_SIZE.
            // It only change the memory controlled by page table.
            current = unsafe {
                &mut *(paddr_to_vaddr(
                    current.paddr() + size_of::<T>() * T::page_index(vaddr, count),
                ) as *mut T)
            };
        }

        let offset_width = (count - 1) * 9 + 12;
        let offset_mask = (1usize << offset_width) - 1;
        Some(current.paddr() + (vaddr & offset_mask))
    }

    pub(crate) fn new(config: PageTableConfig) -> Self {
        let root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        Self {
            root_paddr: root_frame.start_paddr(),
            tables: vec![root_frame],
            config: config,
            _phantom: PhantomData,
        }
    }
}

bitflags::bitflags! {
    pub struct MapProperty: u8 {
        const READ = 0b0001;
        const WRITE = 0b0010;
        const EXEC = 0b0100;
    }
}

impl MapProperty {
    pub fn from<F: PageTableFlagsTrait>(flags: F) -> Self {
        let mut prop = Self::empty();
        if flags.readable() {
            prop |= Self::READ;
        }
        if flags.writable() {
            prop |= Self::WRITE;
        }
        if flags.executable() {
            prop |= Self::EXEC;
        }
        prop
    }
}

pub struct PageTableQueryIter<'a, T, M> {
    page_table: &'a PageTable<T, M>,
    cur_va: Vaddr,
    cur_pt: &'a PageTableFrame<T>,
    cur_level: usize,
}

impl<T, M> Iterator for PageTableQueryIter<'_, T, M> {
    type Item = (Vaddr, Paddr, MapProperty);

    fn next(&mut self) -> Option<Self::Item> {
        todo!()
    }
}

pub struct PageTableFrame<T> {
    frame: VmFrame,
    level: usize,
}

impl<T: PageTableEntryTrait> PageTableFrame<T> {
    pub fn new() -> Self {
        Self(VmAllocOptions::new(1).alloc_single().unwrap())
    }

    pub fn as_slice(&self) -> &[T] {
        unsafe { core::slice::from_raw_parts(self.0.start_vaddr() as *const T, NR_ENTRIES_PER_PAGE) }
    }

    pub fn as_slice_mut(&mut self) -> &mut [T] {
        unsafe { core::slice::from_raw_parts_mut(self.0.start_vaddr() as *mut T, NR_ENTRIES_PER_PAGE) }
    }
}

pub fn init() {
    KERNEL_PAGE_TABLE.call_once(|| {
        #[cfg(target_arch = "x86_64")]
        let (page_directory_base, _) = x86_64::registers::control::Cr3::read();
        SpinLock::new(PageTable {
            root_paddr: page_directory_base.start_address().as_u64() as usize,
            tables: Vec::new(),
            config: PageTableConfig {
                address_width: AddressWidth::Level4,
                supported_translation_depth: 2,
            },
            _phantom: PhantomData,
        })
    });
}

fn range_contains<Idx: PartialOrd<Idx>>(parent: Range<Idx>, child: Range<Idx>) -> bool {
    parent.start <= child.start && parent.end >= child.end
}
