// SPDX-License-Identifier: MPL-2.0

use core::{
    cmp::{max, min},
    num::NonZeroUsize,
    ops::Range,
};

use align_ext::AlignExt;
use ostd::mm::{
    tlb::TlbFlushOp, vm_space::VmItem, CachePolicy, Frame, FrameAllocOptions, PageFlags,
    PageProperty, VmSpace,
};

use super::interval_set::Interval;
use crate::{
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::{perms::VmPerms, util::duplicate_frame, vmo::Vmo},
};

/// Mapping a range of physical pages into a `Vmar`.
///
/// A `VmMapping` can bind with a `Vmo` which can provide physical pages for
/// mapping. Otherwise, it must be an anonymous mapping and will map any empty
/// physical page. A `VmMapping` binding with a `Vmo` is called VMO-backed
/// mapping. Generally, a VMO-backed mapping is a file-backed mapping. Yet
/// there are also some situations where specific pages that are not in a file
/// need to be mapped. e.g:
///  - Mappings to the VDSO data.
///  - Shared anonymous mappings. because the mapped pages need to be retained
///    and shared with other processes.
///
/// Such mappings will also be VMO-backed mappings.
///
/// This type controls the actual mapping in the [`VmSpace`]. It is a linear
/// type and cannot be [`Drop`]. To remove a mapping, use [`Self::unmap`].
#[derive(Debug)]
pub(super) struct VmMapping {
    /// The size of mapping, in bytes. The map size can even be larger than the
    /// size of VMO. Those pages outside VMO range cannot be read or write.
    ///
    /// Zero sized mapping is not allowed. So this field is always non-zero.
    map_size: NonZeroUsize,
    /// The base address relative to the root VMAR where the VMO is mapped.
    map_to_addr: Vaddr,
    /// Specific physical pages that need to be mapped. If this field is
    /// `None`, it means that the mapping is an independent anonymous mapping.
    ///
    /// The start of the virtual address maps to the start of the range
    /// specified in [`MappedVmo`].
    vmo: Option<MappedVmo>,
    /// Whether the mapping is shared.
    ///
    /// The updates to a shared mapping are visible among processes, or carried
    /// through to the underlying file for file-backed shared mappings.
    is_shared: bool,
    /// Whether the mapping needs to handle surrounding pages when handling
    /// page fault.
    handle_page_faults_around: bool,
    /// The permissions of pages in the mapping.
    ///
    /// All pages within the same `VmMapping` have the same permissions.
    perms: VmPerms,
}

impl Interval<Vaddr> for VmMapping {
    fn range(&self) -> Range<Vaddr> {
        self.map_to_addr..self.map_to_addr + self.map_size.get()
    }
}

/***************************** Basic methods *********************************/

impl VmMapping {
    pub(super) fn new(
        map_size: NonZeroUsize,
        map_to_addr: Vaddr,
        vmo: Option<MappedVmo>,
        is_shared: bool,
        handle_page_faults_around: bool,
        perms: VmPerms,
    ) -> Self {
        Self {
            map_size,
            map_to_addr,
            vmo,
            is_shared,
            handle_page_faults_around,
            perms,
        }
    }

    pub(super) fn new_fork(&self) -> Result<VmMapping> {
        Ok(VmMapping {
            vmo: self.vmo.as_ref().map(|vmo| vmo.dup()).transpose()?,
            ..*self
        })
    }

    /// Returns the mapping's start address.
    pub fn map_to_addr(&self) -> Vaddr {
        self.map_to_addr
    }

    /// Returns the mapping's end address.
    pub fn map_end(&self) -> Vaddr {
        self.map_to_addr + self.map_size.get()
    }

    /// Returns the mapping's size.
    pub fn map_size(&self) -> usize {
        self.map_size.get()
    }

    // Returns the permissions of pages in the mapping.
    pub fn perms(&self) -> VmPerms {
        self.perms
    }
}

/****************************** Page faults **********************************/

impl VmMapping {
    pub fn handle_page_fault(
        &self,
        vm_space: &VmSpace,
        page_fault_info: &PageFaultInfo,
    ) -> Result<()> {
        if !self.perms.contains(page_fault_info.required_perms) {
            trace!(
                "self.perms {:?}, page_fault_info.required_perms {:?}, self.range {:?}",
                self.perms,
                page_fault_info.required_perms,
                self.range()
            );
            return_errno_with_message!(Errno::EACCES, "perm check fails");
        }

        let address = page_fault_info.address;

        let page_aligned_addr = address.align_down(PAGE_SIZE);
        let is_write = page_fault_info.required_perms.contains(VmPerms::WRITE);

        if !is_write && self.vmo.is_some() && self.handle_page_faults_around {
            self.handle_page_faults_around(vm_space, address)?;
            return Ok(());
        }

        let mut cursor =
            vm_space.cursor_mut(&(page_aligned_addr..page_aligned_addr + PAGE_SIZE))?;

        match cursor.query().unwrap() {
            VmItem::Mapped {
                va,
                frame,
                mut prop,
            } => {
                if VmPerms::from(prop.flags).contains(page_fault_info.required_perms) {
                    // The page fault is already handled maybe by other threads.
                    // Just flush the TLB and return.
                    TlbFlushOp::Address(va).perform_on_current();
                    return Ok(());
                }
                assert!(is_write);
                // Perform COW if it is a write access to a shared mapping.

                // Skip if the page fault is already handled.
                if prop.flags.contains(PageFlags::W) {
                    return Ok(());
                }

                // If the forked child or parent immediately unmaps the page after
                // the fork without accessing it, we are the only reference to the
                // frame. We can directly map the frame as writable without
                // copying. In this case, the reference count of the frame is 2 (
                // one for the mapping and one for the frame handle itself).
                let only_reference = frame.reference_count() == 2;

                let new_flags = PageFlags::W | PageFlags::ACCESSED | PageFlags::DIRTY;

                if self.is_shared || only_reference {
                    cursor.protect_next(PAGE_SIZE, |p| p.flags |= new_flags);
                    cursor.flusher().issue_tlb_flush(TlbFlushOp::Address(va));
                    cursor.flusher().dispatch_tlb_flush();
                } else {
                    let new_frame = duplicate_frame(&frame)?;
                    prop.flags |= new_flags;
                    cursor.map(new_frame, prop);
                }
            }
            VmItem::NotMapped { .. } => {
                // Map a new frame to the page fault address.

                let (frame, is_readonly) = self.prepare_page(address, is_write)?;

                let vm_perms = {
                    let mut perms = self.perms;
                    if is_readonly {
                        // COW pages are forced to be read-only.
                        perms -= VmPerms::WRITE;
                    }
                    perms
                };

                let mut page_flags = vm_perms.into();
                page_flags |= PageFlags::ACCESSED;
                if is_write {
                    page_flags |= PageFlags::DIRTY;
                }
                let map_prop = PageProperty::new(page_flags, CachePolicy::Writeback);

                cursor.map(frame, map_prop);
            }
        }
        Ok(())
    }

    fn prepare_page(&self, page_fault_addr: Vaddr, write: bool) -> Result<(Frame, bool)> {
        let mut is_readonly = false;
        let Some(vmo) = &self.vmo else {
            return Ok((FrameAllocOptions::new(1).alloc_single()?, is_readonly));
        };

        let page_offset = page_fault_addr.align_down(PAGE_SIZE) - self.map_to_addr;
        let Ok(page) = vmo.get_committed_frame(page_offset) else {
            if !self.is_shared {
                // The page index is outside the VMO. This is only allowed in private mapping.
                return Ok((FrameAllocOptions::new(1).alloc_single()?, is_readonly));
            } else {
                return_errno_with_message!(
                    Errno::EFAULT,
                    "could not find a corresponding physical page"
                );
            }
        };

        if !self.is_shared && write {
            // Write access to private VMO-backed mapping. Performs COW directly.
            Ok((duplicate_frame(&page)?, is_readonly))
        } else {
            // Operations to shared mapping or read access to private VMO-backed mapping.
            // If read access to private VMO-backed mapping triggers a page fault,
            // the map should be readonly. If user next tries to write to the frame,
            // another page fault will be triggered which will performs a COW (Copy-On-Write).
            is_readonly = !self.is_shared;
            Ok((page, is_readonly))
        }
    }

    fn handle_page_faults_around(&self, vm_space: &VmSpace, page_fault_addr: Vaddr) -> Result<()> {
        const SURROUNDING_PAGE_NUM: usize = 16;
        const SURROUNDING_PAGE_ADDR_MASK: usize = !(SURROUNDING_PAGE_NUM * PAGE_SIZE - 1);

        let vmo = self.vmo.as_ref().unwrap();
        let around_page_addr = page_fault_addr & SURROUNDING_PAGE_ADDR_MASK;
        let size = min(vmo.size(), self.map_size.get());

        let start_addr = max(around_page_addr, self.map_to_addr);
        let end_addr = min(
            start_addr + SURROUNDING_PAGE_NUM * PAGE_SIZE,
            self.map_to_addr + size,
        );

        let vm_perms = self.perms - VmPerms::WRITE;
        let mut cursor = vm_space.cursor_mut(&(start_addr..end_addr))?;
        let operate = move |commit_fn: &mut dyn FnMut() -> Result<Frame>| {
            if let VmItem::NotMapped { .. } = cursor.query().unwrap() {
                // We regard all the surrounding pages as accessed, no matter
                // if it is really so. Then the hardware won't bother to update
                // the accessed bit of the page table on following accesses.
                let page_flags = PageFlags::from(vm_perms) | PageFlags::ACCESSED;
                let page_prop = PageProperty::new(page_flags, CachePolicy::Writeback);
                let frame = commit_fn()?;
                cursor.map(frame, page_prop);
            } else {
                let next_addr = cursor.virt_addr() + PAGE_SIZE;
                if next_addr < end_addr {
                    let _ = cursor.jump(next_addr);
                }
            }
            Ok(())
        };

        let start_offset = start_addr - self.map_to_addr;
        let end_offset = end_addr - self.map_to_addr;
        vmo.operate_on_range(&(start_offset..end_offset), operate)?;

        Ok(())
    }
}

/**************************** Transformations ********************************/

impl VmMapping {
    /// Enlarges the mapping by `extra_size` bytes to the high end.
    pub fn enlarge(self, extra_size: usize) -> Self {
        Self {
            map_size: NonZeroUsize::new(self.map_size.get() + extra_size).unwrap(),
            ..self
        }
    }

    /// Splits the mapping at the specified address.
    ///
    /// The address must be within the mapping and page-aligned. The address
    /// must not be either the start or the end of the mapping.
    fn split(self, at: Vaddr) -> Result<(Self, Self)> {
        debug_assert!(self.map_to_addr < at && at < self.map_end());
        debug_assert!(at % PAGE_SIZE == 0);

        let (mut l_vmo, mut r_vmo) = (None, None);

        if let Some(vmo) = self.vmo {
            let at_offset = vmo.range.start + at - self.map_to_addr;

            let l_range = vmo.range.start..at_offset;
            let r_range = at_offset..vmo.range.end;

            l_vmo = Some(MappedVmo::new(vmo.vmo.dup()?, l_range));
            r_vmo = Some(MappedVmo::new(vmo.vmo.dup()?, r_range));
        }

        let left_size = at - self.map_to_addr;
        let right_size = self.map_size.get() - left_size;
        let left = Self {
            map_to_addr: self.map_to_addr,
            map_size: NonZeroUsize::new(left_size).unwrap(),
            vmo: l_vmo,
            ..self
        };
        let right = Self {
            map_to_addr: at,
            map_size: NonZeroUsize::new(right_size).unwrap(),
            vmo: r_vmo,
            ..self
        };

        Ok((left, right))
    }

    /// Splits the mapping at the specified address.
    ///
    /// There are four conditions:
    /// 1. |-outside `range`-| + |------------within `range`------------|
    /// 2. |------------within `range`------------| + |-outside `range`-|
    /// 3. |-outside `range`-| + |-within `range`-| + |-outside `range`-|
    /// 4. |----------------------within `range` -----------------------|
    ///
    /// Returns (left outside, within, right outside) if successful.
    ///
    /// # Panics
    ///
    /// Panics if the mapping does not contain the range, or if the start or
    /// end of the range is not page-aligned.
    pub fn split_range(self, range: &Range<Vaddr>) -> Result<(Option<Self>, Self, Option<Self>)> {
        let mapping_range = self.range();
        if range.start <= mapping_range.start && mapping_range.end <= range.end {
            // Condition 4.
            return Ok((None, self, None));
        } else if mapping_range.start < range.start {
            let (left, within) = self.split(range.start).unwrap();
            if range.end < mapping_range.end {
                // Condition 3.
                let (within, right) = within.split(range.end).unwrap();
                return Ok((Some(left), within, Some(right)));
            } else {
                // Condition 1.
                return Ok((Some(left), within, None));
            }
        } else if mapping_range.contains(&range.end) {
            // Condition 2.
            let (within, right) = self.split(range.end).unwrap();
            return Ok((None, within, Some(right)));
        }
        panic!("The mapping does not contain the splitting range.");
    }
}

/************************** VM Space operations ******************************/

impl VmMapping {
    /// Unmaps the mapping from the VM space.
    pub(super) fn unmap(self, vm_space: &VmSpace) -> Result<()> {
        let range = self.range();
        let mut cursor = vm_space.cursor_mut(&range)?;
        cursor.unmap(range.len());

        Ok(())
    }

    /// Change the perms of the mapping.
    pub(super) fn protect(self, vm_space: &VmSpace, perms: VmPerms) -> Self {
        let range = self.range();

        let mut cursor = vm_space.cursor_mut(&range).unwrap();

        let op = |p: &mut PageProperty| p.flags = perms.into();
        while cursor.virt_addr() < range.end {
            if let Some(va) = cursor.protect_next(range.end - cursor.virt_addr(), op) {
                cursor.flusher().issue_tlb_flush(TlbFlushOp::Range(va));
            } else {
                break;
            }
        }
        cursor.flusher().dispatch_tlb_flush();

        Self { perms, ..self }
    }
}

/// A wrapper that represents a mapped [`Vmo`] and provide required functionalities
/// that need to be provided to mappings from the VMO.
#[derive(Debug)]
pub(super) struct MappedVmo {
    vmo: Vmo,
    /// Represents the accessible range in the VMO for mappings.
    range: Range<usize>,
}

impl MappedVmo {
    /// Creates a `MappedVmo` used for mapping.
    pub(super) fn new(vmo: Vmo, range: Range<usize>) -> Self {
        Self { vmo, range }
    }

    fn size(&self) -> usize {
        self.range.len()
    }

    /// Gets the committed frame at the input offset in the mapped VMO.
    ///
    /// If the VMO has not committed a frame at this index, it will commit
    /// one first and return it.
    fn get_committed_frame(&self, page_offset: usize) -> Result<Frame> {
        debug_assert!(page_offset < self.range.len());
        debug_assert!(page_offset % PAGE_SIZE == 0);
        self.vmo.commit_page(self.range.start + page_offset)
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    fn operate_on_range<F>(&self, range: &Range<usize>, operate: F) -> Result<()>
    where
        F: FnMut(&mut dyn FnMut() -> Result<Frame>) -> Result<()>,
    {
        debug_assert!(range.start < self.range.len());
        debug_assert!(range.end <= self.range.len());

        let range = self.range.start + range.start..self.range.start + range.end;

        self.vmo.operate_on_range(&range, operate)
    }

    /// Duplicates the capability.
    pub fn dup(&self) -> Result<Self> {
        Ok(Self {
            vmo: self.vmo.dup()?,
            range: self.range.clone(),
        })
    }
}
