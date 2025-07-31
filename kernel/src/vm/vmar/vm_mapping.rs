// SPDX-License-Identifier: MPL-2.0

use core::{
    cmp::{max, min},
    num::NonZeroUsize,
    ops::Range,
};

use align_ext::AlignExt;
use ostd::{
    mm::{
        tlb::TlbFlushOp, CachePolicy, FrameAllocOptions, PageFlags, PageProperty, UFrame, VmSpace,
    },
    task::disable_preempt,
};

use super::{interval_set::Interval, RssDelta, RssType};
use crate::{
    fs::utils::Inode,
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::{
        perms::VmPerms,
        util::duplicate_frame,
        vmar::is_intersected,
        vmo::{CommitFlags, Vmo, VmoCommitError},
    },
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
pub struct VmMapping {
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
    /// The inode of the file that backs the mapping.
    ///
    /// If the inode is `Some`, it means that the mapping is file-backed.
    /// And the `vmo` field must be the page cache of the inode.
    inode: Option<Arc<dyn Inode>>,
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
        inode: Option<Arc<dyn Inode>>,
        is_shared: bool,
        handle_page_faults_around: bool,
        perms: VmPerms,
    ) -> Self {
        Self {
            map_size,
            map_to_addr,
            vmo,
            inode,
            is_shared,
            handle_page_faults_around,
            perms,
        }
    }

    pub(super) fn new_fork(&self) -> Result<VmMapping> {
        Ok(VmMapping {
            vmo: self.vmo.as_ref().map(|vmo| vmo.dup()).transpose()?,
            inode: self.inode.clone(),
            ..*self
        })
    }

    pub(super) fn clone_for_remap_at(&self, va: Vaddr) -> Result<VmMapping> {
        let mut vm_mapping = self.new_fork()?;
        vm_mapping.map_to_addr = va;
        Ok(vm_mapping)
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

    /// Returns the permissions of pages in the mapping.
    pub fn perms(&self) -> VmPerms {
        self.perms
    }

    /// Returns the inode of the file that backs the mapping.
    pub fn inode(&self) -> Option<&Arc<dyn Inode>> {
        self.inode.as_ref()
    }

    /// Returns the mapping's RSS type.
    pub fn rss_type(&self) -> RssType {
        if self.vmo.is_none() {
            RssType::RSS_ANONPAGES
        } else {
            RssType::RSS_FILEPAGES
        }
    }
}

/****************************** Page faults **********************************/

impl VmMapping {
    /// Handles a page fault.
    pub(super) fn handle_page_fault(
        &self,
        vm_space: &VmSpace,
        page_fault_info: &PageFaultInfo,
        rss_delta: &mut RssDelta,
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

        let page_aligned_addr = page_fault_info.address.align_down(PAGE_SIZE);
        let is_write = page_fault_info.required_perms.contains(VmPerms::WRITE);

        if !is_write && self.vmo.is_some() && self.handle_page_faults_around {
            let res = self.handle_page_faults_around(
                vm_space,
                page_aligned_addr,
                page_fault_info.required_perms,
                rss_delta,
            );

            // Errors caused by the "around" pages should be ignored, so here we
            // only return the error if the faulting page is still not mapped.
            if res.is_err() {
                let preempt_guard = disable_preempt();
                let mut cursor = vm_space.cursor(
                    &preempt_guard,
                    &(page_aligned_addr..page_aligned_addr + PAGE_SIZE),
                )?;
                if let (_, Some((_, _))) = cursor.query().unwrap() {
                    return Ok(());
                }
            }

            return res;
        }

        self.handle_single_page_fault(
            vm_space,
            page_aligned_addr,
            page_fault_info.required_perms,
            rss_delta,
        )
    }

    fn handle_single_page_fault(
        &self,
        vm_space: &VmSpace,
        page_aligned_addr: Vaddr,
        required_perms: VmPerms,
        rss_delta: &mut RssDelta,
    ) -> Result<()> {
        'retry: loop {
            let preempt_guard = disable_preempt();
            let mut cursor = vm_space.cursor_mut(
                &preempt_guard,
                &(page_aligned_addr..page_aligned_addr + PAGE_SIZE),
            )?;

            let (va, item) = cursor.query().unwrap();
            let is_write = required_perms.contains(VmPerms::WRITE);
            match item {
                Some((frame, mut prop)) => {
                    if VmPerms::from(prop.flags).contains(required_perms) {
                        // The page fault is already handled maybe by other threads.
                        // Just flush the TLB and return.
                        TlbFlushOp::Range(va).perform_on_current();
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
                        cursor.protect_next(PAGE_SIZE, |flags, _cache| {
                            *flags |= new_flags;
                        });
                        cursor.flusher().issue_tlb_flush(TlbFlushOp::Range(va));
                        cursor.flusher().dispatch_tlb_flush();
                    } else {
                        let new_frame = duplicate_frame(&frame)?;
                        prop.flags |= new_flags;
                        cursor.map(new_frame.into(), prop);
                        rss_delta.add(self.rss_type(), 1);
                    }
                    cursor.flusher().sync_tlb_flush();
                }
                None => {
                    // Map a new frame to the page fault address.
                    let (frame, is_readonly) = match self.prepare_page(page_aligned_addr, is_write)
                    {
                        Ok((frame, is_readonly)) => (frame, is_readonly),
                        Err(VmoCommitError::Err(e)) => return Err(e),
                        Err(VmoCommitError::NeedIo(index)) => {
                            drop(cursor);
                            drop(preempt_guard);
                            self.vmo
                                .as_ref()
                                .unwrap()
                                .commit_on(index, CommitFlags::empty())?;
                            continue 'retry;
                        }
                    };

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
                    let map_prop = PageProperty::new_user(page_flags, CachePolicy::Writeback);

                    cursor.map(frame, map_prop);
                    rss_delta.add(self.rss_type(), 1);
                }
            }
            break 'retry;
        }

        Ok(())
    }

    fn prepare_page(
        &self,
        page_aligned_addr: Vaddr,
        write: bool,
    ) -> core::result::Result<(UFrame, bool), VmoCommitError> {
        let mut is_readonly = false;
        let Some(vmo) = &self.vmo else {
            return Ok((FrameAllocOptions::new().alloc_frame()?.into(), is_readonly));
        };

        let page_offset = page_aligned_addr - self.map_to_addr;
        if !self.is_shared && page_offset >= vmo.valid_size() {
            // The page index is outside the VMO. This is only allowed in private mapping.
            return Ok((FrameAllocOptions::new().alloc_frame()?.into(), is_readonly));
        }

        let page = vmo.get_committed_frame(page_offset)?;
        if !self.is_shared && write {
            // Write access to private VMO-backed mapping. Performs COW directly.
            Ok((duplicate_frame(&page)?.into(), is_readonly))
        } else {
            // Operations to shared mapping or read access to private VMO-backed mapping.
            // If read access to private VMO-backed mapping triggers a page fault,
            // the map should be readonly. If user next tries to write to the frame,
            // another page fault will be triggered which will performs a COW (Copy-On-Write).
            is_readonly = !self.is_shared;
            Ok((page, is_readonly))
        }
    }

    /// Handles a page fault and maps additional surrounding pages.
    fn handle_page_faults_around(
        &self,
        vm_space: &VmSpace,
        page_aligned_addr: Vaddr,
        required_perms: VmPerms,
        mut rss_delta: &mut RssDelta,
    ) -> Result<()> {
        const SURROUNDING_PAGE_NUM: usize = 16;
        const SURROUNDING_PAGE_ADDR_MASK: usize = !(SURROUNDING_PAGE_NUM * PAGE_SIZE - 1);

        let vmo = self.vmo.as_ref().unwrap();
        let around_page_addr = page_aligned_addr & SURROUNDING_PAGE_ADDR_MASK;
        let size = min(vmo.valid_size(), self.map_size.get());
        let mut start_addr = max(around_page_addr, self.map_to_addr);
        let end_addr = min(
            start_addr + SURROUNDING_PAGE_NUM * PAGE_SIZE,
            self.map_to_addr + size,
        );

        // The page fault address falls outside the VMO bounds.
        // Only a single page fault is handled in this situation.
        if end_addr <= page_aligned_addr {
            return self.handle_single_page_fault(
                vm_space,
                page_aligned_addr,
                required_perms,
                rss_delta,
            );
        }

        let vm_perms = self.perms - VmPerms::WRITE;

        'retry: loop {
            let preempt_guard = disable_preempt();
            let mut cursor = vm_space.cursor_mut(&preempt_guard, &(start_addr..end_addr))?;

            let rss_delta_ref = &mut rss_delta;
            let operate =
                move |commit_fn: &mut dyn FnMut()
                    -> core::result::Result<UFrame, VmoCommitError>| {
                    if let (_, None) = cursor.query().unwrap() {
                        // We regard all the surrounding pages as accessed, no matter
                        // if it is really so. Then the hardware won't bother to update
                        // the accessed bit of the page table on following accesses.
                        let page_flags = PageFlags::from(vm_perms) | PageFlags::ACCESSED;
                        let page_prop = PageProperty::new_user(page_flags, CachePolicy::Writeback);
                        let frame = commit_fn()?;
                        cursor.map(frame, page_prop);
                        rss_delta_ref.add(self.rss_type(), 1);
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
            match vmo.try_operate_on_range(&(start_offset..end_offset), operate) {
                Ok(_) => return Ok(()),
                Err(VmoCommitError::NeedIo(index)) => {
                    drop(preempt_guard);
                    vmo.commit_on(index, CommitFlags::empty())?;
                    start_addr = (index * PAGE_SIZE - vmo.offset) + self.map_to_addr;
                    continue 'retry;
                }
                Err(VmoCommitError::Err(e)) => return Err(e),
            }
        }
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
    pub fn split(self, at: Vaddr) -> Result<(Self, Self)> {
        debug_assert!(self.map_to_addr < at && at < self.map_end());
        debug_assert!(at % PAGE_SIZE == 0);

        let (mut l_vmo, mut r_vmo) = (None, None);

        if let Some(vmo) = self.vmo {
            let at_offset = vmo.offset + (at - self.map_to_addr);

            l_vmo = Some(vmo.dup()?);
            r_vmo = Some(MappedVmo::new(vmo.vmo.dup()?, at_offset));
        }

        let left_size = at - self.map_to_addr;
        let right_size = self.map_size.get() - left_size;
        let left = Self {
            map_to_addr: self.map_to_addr,
            map_size: NonZeroUsize::new(left_size).unwrap(),
            vmo: l_vmo,
            inode: self.inode.clone(),
            ..self
        };
        let right = Self {
            map_to_addr: at,
            map_size: NonZeroUsize::new(right_size).unwrap(),
            vmo: r_vmo,
            inode: self.inode,
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
    pub fn split_range(self, range: &Range<Vaddr>) -> (Option<Self>, Self, Option<Self>) {
        let mapping_range = self.range();
        if range.start <= mapping_range.start && mapping_range.end <= range.end {
            // Condition 4.
            (None, self, None)
        } else if mapping_range.start < range.start {
            let (left, within) = self.split(range.start).unwrap();
            if range.end < mapping_range.end {
                // Condition 3.
                let (within, right) = within.split(range.end).unwrap();
                (Some(left), within, Some(right))
            } else {
                // Condition 1.
                (Some(left), within, None)
            }
        } else if mapping_range.contains(&range.end) {
            // Condition 2.
            let (within, right) = self.split(range.end).unwrap();
            (None, within, Some(right))
        } else {
            panic!("The mapping does not contain the splitting range");
        }
    }

    /// Attempts to merge `self` with the given `vm_mapping` if they are
    /// adjacent and compatible.
    ///
    /// Two mappings are considered *adjacent* if the end address of `self`
    /// equals to the start address of `vm_mapping`, or vice versa.
    ///
    /// Two mappings are considered *compatible* if all of the following
    /// conditions are met:
    /// - They have the same access permissions.
    /// - They are both anonymous or share the same backing file.
    /// - Their file offsets are contiguous if file-backed.
    /// - Other attributes (e.g., shared/private flags, whether need to handle
    ///   page faults around, etc.) must also match.
    ///
    /// This method returns:
    /// - the merged mapping along with the address of the mapping
    ///   to be removed if successful.
    /// - the original `self` and a `None` otherwise.
    pub fn try_merge_with(self, vm_mapping: &VmMapping) -> (Self, Option<Vaddr>) {
        debug_assert!(!is_intersected(&self.range(), &vm_mapping.range()));

        let (left, right) = if self.map_to_addr < vm_mapping.map_to_addr {
            (&self, vm_mapping)
        } else {
            (vm_mapping, &self)
        };

        if let Some(merged) = try_merge(left, right) {
            (merged, Some(vm_mapping.map_to_addr))
        } else {
            (self, None)
        }
    }
}

/************************** VM Space operations ******************************/

impl VmMapping {
    /// Unmaps the mapping from the VM space,
    /// and returns the number of unmapped pages.
    pub(super) fn unmap(self, vm_space: &VmSpace) -> usize {
        let preempt_guard = disable_preempt();
        let range = self.range();
        let mut cursor = vm_space.cursor_mut(&preempt_guard, &range).unwrap();

        let num_unmapped = cursor.unmap(range.len());
        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        num_unmapped
    }

    /// Change the perms of the mapping.
    pub(super) fn protect(self, vm_space: &VmSpace, perms: VmPerms) -> Self {
        let preempt_guard = disable_preempt();
        let range = self.range();
        let mut cursor = vm_space.cursor_mut(&preempt_guard, &range).unwrap();

        let op = |flags: &mut PageFlags, _cache: &mut CachePolicy| *flags = perms.into();
        while cursor.virt_addr() < range.end {
            if let Some(va) = cursor.protect_next(range.end - cursor.virt_addr(), op) {
                cursor.flusher().issue_tlb_flush(TlbFlushOp::Range(va));
            } else {
                break;
            }
        }
        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        Self { perms, ..self }
    }
}

/// A wrapper that represents a mapped [`Vmo`] and provide required functionalities
/// that need to be provided to mappings from the VMO.
#[derive(Debug)]
pub(super) struct MappedVmo {
    vmo: Vmo,
    /// Represents the mapped offset in the VMO for the mapping.
    offset: usize,
}

impl MappedVmo {
    /// Creates a `MappedVmo` used for the mapping.
    pub(super) fn new(vmo: Vmo, offset: usize) -> Self {
        Self { vmo, offset }
    }

    /// Returns the **valid** size of the `MappedVmo`.
    ///
    /// The **valid** size of a `MappedVmo` is the size of its accessible range
    /// that actually falls within the bounds of the underlying VMO.
    fn valid_size(&self) -> usize {
        let vmo_size = self.vmo.size();
        (self.offset..vmo_size).len()
    }

    /// Gets the committed frame at the input offset in the mapped VMO.
    ///
    /// If the VMO has not committed a frame at this index, it will commit
    /// one first and return it. If the commit operation needs to perform I/O,
    /// it will return a [`VmoCommitError::NeedIo`].
    fn get_committed_frame(
        &self,
        page_offset: usize,
    ) -> core::result::Result<UFrame, VmoCommitError> {
        debug_assert!(page_offset % PAGE_SIZE == 0);
        self.vmo.try_commit_page(self.offset + page_offset)
    }

    /// Commits a page at a specific page index.
    ///
    /// This method may involve I/O operations if the VMO needs to fecth
    /// a page from the underlying page cache.
    pub fn commit_on(&self, page_idx: usize, commit_flags: CommitFlags) -> Result<UFrame> {
        self.vmo.commit_on(page_idx, commit_flags)
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    ///
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    ///
    /// Once a commit operation needs to perform I/O, it will return a [`VmoCommitError::NeedIo`].
    fn try_operate_on_range<F>(
        &self,
        range: &Range<usize>,
        operate: F,
    ) -> core::result::Result<(), VmoCommitError>
    where
        F: FnMut(
            &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>,
        ) -> core::result::Result<(), VmoCommitError>,
    {
        let range = self.offset + range.start..self.offset + range.end;
        self.vmo.try_operate_on_range(&range, operate)
    }

    /// Duplicates the capability.
    pub fn dup(&self) -> Result<Self> {
        Ok(Self {
            vmo: self.vmo.dup()?,
            offset: self.offset,
        })
    }
}

/// Attempts to merge two [`VmMapping`]s into a single mapping if they are
/// adjacent and compatible.
///
/// - Returns the merged [`VmMapping`] if successful. The caller should
///   remove the original mappings before inserting the merged mapping
///   into the [`Vmar`].
/// - Returns `None` otherwise.
fn try_merge(left: &VmMapping, right: &VmMapping) -> Option<VmMapping> {
    let is_adjacent = left.map_end() == right.map_to_addr();
    let is_type_equal = left.is_shared == right.is_shared
        && left.handle_page_faults_around == right.handle_page_faults_around
        && left.perms == right.perms;

    if !is_adjacent || !is_type_equal {
        return None;
    }

    let vmo = match (&left.vmo, &right.vmo) {
        (None, None) => None,
        (Some(l_vmo), Some(r_vmo)) if Arc::ptr_eq(&l_vmo.vmo.0, &r_vmo.vmo.0) => {
            let is_offset_contiguous = l_vmo.offset + left.map_size() == r_vmo.offset;
            if !is_offset_contiguous {
                return None;
            }
            Some(l_vmo.dup().ok()?)
        }
        _ => return None,
    };

    let map_size = NonZeroUsize::new(left.map_size() + right.map_size()).unwrap();

    Some(VmMapping {
        map_size,
        vmo,
        inode: left.inode.clone(),
        ..*left
    })
}
