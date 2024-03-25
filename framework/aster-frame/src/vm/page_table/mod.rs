// SPDX-License-Identifier: MPL-2.0

use alloc::{sync::Arc, boxed::Box};
use core::{fmt::Debug, marker::PhantomData, ops::Range};

use spin::Once;

use super::{paddr_to_vaddr, Paddr, Vaddr};
use crate::{
    arch::mm::{PageTableConsts, PageTableEntry}, boot::memory_region::MemoryRegionType, sync::SpinLock, vm::{VmAllocOptions, VmFrame, BASE_PAGE_SIZE}
};

mod properties;
pub use properties::*;
mod cursor;
use cursor::*;

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

/// A page table instance.
#[derive(Clone, Debug)]
pub struct PageTable<M: PageTableMode, E: PageTableEntryTrait = PageTableEntry, C: PageTableConstsTrait = PageTableConsts> {
    root_frame: Arc<PageTableFrame<E, C>>,
    _phantom: PhantomData<M>,
}

struct PageTableFrame<E, C: PageTableConstsTrait> {
    pub inner: VmFrame,
    // TODO: we may have a 2x space optimization here:
    // just design a typed VmFrame box like VmFrameBox<T> which is basically
    // `[T; BASE_PAGE_SIZE / size_of::<T>()]` at memory and a
    // `Arc<Unique<[T; BASE_PAGE_SIZE / size_of::<T>()]>>` as the type.
    // Also, a `T` should be unsafely regarded as a enum that may hold a frame.
    pub child: Option<Box<[Option<Arc<PageTableFrame<E, C>>>; nr_entries_per_frame::<C>()]>>,
    _phantom: PhantomData<(E, C)>,
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> PageTable<UserMode, E, C> {
    pub fn map(
        &mut self,
        vaddr: Vaddr,
        frame: &VmFrame,
        prop: MapProperty,
    ) -> Result<(), PageTableError> {
        if !range_contains(UserMode::VADDR_RANGE, vaddr..vaddr + BASE_PAGE_SIZE) {
            return Err(PageTableError::InvalidVaddr);
        }
        let from = vaddr..vaddr + BASE_PAGE_SIZE;
        let to = frame.start_paddr()..frame.start_paddr() + BASE_PAGE_SIZE;
        // Safety:
        // 1. The vaddr belongs to user mode program and does not affect the kernel mapping.
        // 2. The area where the physical address islocated at untyped memory and does not affect kernel security.
        unsafe { self.do_map(from, Some((&[to].into_iter(), prop))) }
    }
}

impl<E: PageTableEntryTrait, C: PageTableConstsTrait> PageTable<KernelMode, E, C> {
    /// Create a new user page table.
    /// 
    /// This should be the only way to create a user page table, that is
    /// to fork the kernel page table with all the kernel mappings shared.
    pub fn fork(&self) -> PageTable<E, UserMode> {
        let root_frame = VmAllocOptions::new(1).alloc_single().unwrap();
        // Safety: The root_paddr is the root of a valid page table and
        // it does not overlap with the new page.
        unsafe {
            let src = self.root_frame.inner.start_paddr() as *const E;
            let dst = root_frame.start_paddr() as *mut E;
            core::ptr::copy_nonoverlapping(src, dst, nr_entries_per_frame::<C>());
        }
        PageTable::<E, UserMode> {
            root_frame: Arc::new(PageTableFrame::<E, C> {
                inner: root_frame,
                child: self.child.clone(),
                _phantom: PhantomData,
            }),
            _phantom: PhantomData,
        }
    }

    pub unsafe fn map_addr(
        &mut self,
        from: Range<Vaddr>,
        to: Range<Paddr>,
        prop: MapProperty,
    ) -> Result<(), PageTableError> {
        if !range_contains(KernelMode::VADDR_RANGE, from) || from.len() != to.len() {
            return Err(PageTableError::InvalidVaddr);
        }
        self.do_map(from, Some((&[to].into_iter(), prop)))
    }
}

impl<M: PageTableMode, E: PageTableEntryTrait, C: PageTableConstsTrait> PageTable<M, E, C> {
    /// Mapping `vaddr` to `paddr` with flags.
    /// 
    /// This function can be used to map, unmap or protect virtual memory ranges.
    /// The `op` function is for you to customize the operation on the properties
    /// of the range. The argument `F` is the properties on the page table entry
    /// before the mapping operation, and the return value is the properties you
    /// want after the mapping operation.
    /// 
    /// The argument `to` allows you to map the continuous range to several
    /// discontinuous ranges. The caller must ensure that the sum of the sizes of
    /// the ranges in `to` is equal to the size of the range in `from`. If `to`
    /// is `None`, the range in `from` will be unmapped.
    /// 
    /// The function will map as more huge pages as possible, and it will split
    /// the huge pages into smaller pages if necessary. If the input range is large,
    /// the resulting mappings may look like this (if very huge pages supported):
    /// 
    /// ```text
    /// start                                                             end
    ///   |----|----------------|--------------------------------|----|----|
    ///    base      huge                     very huge           base base
    ///    4KiB      2MiB                       1GiB              4KiB  4KiB
    /// ```
    /// 
    /// In practice it is suggested to use simple wrappers for this API that maps
    /// frames for safety and conciseness.
    /// 
    /// # Safety
    /// 
    /// This function manipulates the page table directly, and it is unsafe because
    /// it may cause undefined behavior if the caller does not ensure that the
    /// mapped address is valid and the page table is not corrupted if it is used
    /// by the kernel.
    pub unsafe fn do_map(
        &mut self,
        from: Range<Vaddr>,
        to: Option<(impl IntoIterator<Item = Range<Paddr>>, MapProperty)>,
    ) -> Result<(), PageTableError>
    {
        if from.start % C::BASE_PAGE_SIZE != 0 || from.end % C::BASE_PAGE_SIZE != 0 {
            return Err(PageTableError::InvalidVaddr);
        }

        let mut cursor = PageTableCursor::<E, C, M>::new(self, from.start);

        if let (to, prop) = to {
            // Do map.
            for par in to {
                if par.start % C::BASE_PAGE_SIZE != 0 || par.end % C::BASE_PAGE_SIZE != 0 {
                    return Err(PageTableError::InvalidVaddr);
                }
                cursor.map_contiguous(par.len(), Some((par.start, prop)));
            }
        } else {
            // Do unmap.
            cursor.map_contiguous(from.len(), None);
        }

        Ok(())
    }

    pub fn root_paddr(&self) -> Paddr {
        self.root_frame.inner.start_paddr()
    }

    /// A software emulation of the address translation process.
    pub fn translate(&self, vaddr: Vaddr) -> Option<Paddr> {
        // Safety: The root frame is a valid page table frame so the address is valid.
        unsafe { page_walk::<E, C>(self.root_paddr(), vaddr) }
    }
}

/// Translate a virtual address to a physical address using the page table by
/// providing the value of root page table frame.
/// 
/// # Safety
/// 
/// The caller must ensure that the root_paddr is a valid pointer to the root
/// page table frame.
pub(super) unsafe fn page_walk<E: PageTableEntryTrait, C: PageTableConstsTrait>(root_paddr: Paddr, vaddr: Vaddr) -> Option<Paddr> {
    let mut cur_level = C::NR_LEVELS;
    let mut cur_pte = {
        let frame_addr = paddr_to_vaddr(root_paddr);
        let offset = in_frame_index::<C>(vaddr, cur_level);
        // Safety: The offset does not exceed the value of BASE_PAGE_SIZE.
        unsafe { &*(frame_addr as *const E).add(offset) }
    };

    while cur_level > 1 {
        if !cur_pte.is_valid() {
            return None;
        }
        if cur_pte.is_huge() {
            debug_assert!(cur_level <= C::HIGHEST_TRANSLATION_LEVEL);
            break;
        }
        cur_level -= 1;
        cur_pte = {
            let frame_addr = paddr_to_vaddr(cur_pte.paddr());
            let offset = in_frame_index::<C>(vaddr, cur_level);
            // Safety: The offset does not exceed the value of BASE_PAGE_SIZE.
            unsafe { &*(frame_addr as *const E).add(offset) }
        };
    }

    Some(cur_pte.paddr() + (vaddr & (page_size::<C>(cur_level) - 1)))
}

pub static KERNEL_PAGE_TABLE: Once<SpinLock<PageTable<PageTableEntry, PageTableConsts, KernelMode>>> = Once::new();

pub fn init() {
    KERNEL_PAGE_TABLE.call_once(|| {
        let mut kpt = PageTable::<PageTableEntry, PageTableConsts, KernelMode> {
            root_frame: Arc::new(PageTableFrame::<PageTableEntry, PageTableConsts> {
                inner: VmAllocOptions::new(1).alloc_single().unwrap(),
                child: None,
                _phantom: PhantomData,
            }),
            _phantom: PhantomData,
        };
        let regions = crate::boot::memory_regions();
        // Do linear mappings for the kernel.
        let linear_mapping_size = {
            let mut end = 0;
            for r in regions {
                end = end.max(r.base() + r.len());
            }
            end
        };
        use super::LINEAR_MAPPING_BASE_VADDR;
        let from = LINEAR_MAPPING_BASE_VADDR..LINEAR_MAPPING_BASE_VADDR + linear_mapping_size;
        let to = 0..linear_mapping_size;
        let prop = MapProperty {
            flags: MapFlags::READ | MapFlags::WRITE | MapFlags::GLOBAL,
            cache: MapCachePolicy::WriteBack,
        };
        // Safety: we are doing the linear mapping for the kernel.
        unsafe { kpt.map_addr(from, to, prop); }
        // Map for the kernel code itself.
        // TODO: set permissions for each segments in the kernel.
        let region = regions.iter().find(|r| r.typ() == MemoryRegionType::Kernel).unwrap();
        let offset = super::kernel_loaded_offset();
        let from = region.base()..region.base() + region.len();
        let to = from.start + offset..from.end + offset;
        let prop = MapProperty {
            flags: MapFlags::READ | MapFlags::WRITE | MapFlags::EXEC | MapFlags::GLOBAL,
            cache: MapCachePolicy::WriteBack,
        };
        // Safety: we are doing mappings for the kernel.
        unsafe { kpt.map_addr(from, to, prop); }
        SpinLock::new(kpt)
    });
}

fn range_contains<Idx: PartialOrd<Idx>>(parent: Range<Idx>, child: Range<Idx>) -> bool {
    parent.start <= child.start && parent.end >= child.end
}
