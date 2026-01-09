// SPDX-License-Identifier: MPL-2.0

use core::{
    cmp::{max, min},
    num::NonZeroUsize,
    ops::Range,
};

use align_ext::AlignExt;
use aster_util::printer::VmPrinter;
use ostd::{
    io::IoMem,
    mm::{
        CachePolicy, Frame, FrameAllocOptions, PageFlags, PageProperty, UFrame, VmSpace,
        io_util::HasVmReaderWriter, tlb::TlbFlushOp, vm_space::VmQueriedItem,
    },
    task::disable_preempt,
};

use super::{RssType, interval_set::Interval, util::is_intersected, vmar_impls::RssDelta};
use crate::{
    fs::utils::Inode,
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::{
        perms::VmPerms,
        vmo::{CommitFlags, Vmo, VmoCommitError},
    },
};

/// A memory mapping for a range of virtual addresses in a [`Vmar`].
///
/// A `VmMapping` can be bound with a [`Vmo`] which can provide physical pages
/// for the mapping. Such mappings are called VMO-backed mappings. Generally, a
/// VMO-backed mapping is a file-backed mapping. There are also some exceptions
/// where the pages that need to be mapped are not in a file, but the mappings
/// are still considered as VMO-backed, e.g.,
///  - Mappings to the vDSO data.
///  - Shared anonymous mappings, where the mapped pages need to be retained and
///    shared with other processes.
///
/// Otherwise, the mapping may not be backed by a VMO, in which case it will be
/// private and anonymous. Such mappings will map to newly allocated, zeroed
/// physical pages.
///
/// This type controls the actual mapping in the [`VmSpace`]. It is a linear
/// type and cannot be [`Drop`]. To remove a mapping, use [`Self::unmap`].
///
/// [`Vmar`]: crate::vm::vmar::Vmar
#[derive(Debug)]
pub struct VmMapping {
    /// The size of mapping, in bytes. The map size can even be larger than the
    /// size of VMO. Those pages outside VMO range cannot be read or write.
    ///
    /// Zero sized mapping is not allowed. So this field is always non-zero.
    map_size: NonZeroUsize,
    /// The base address relative to the VMAR where the VMO is mapped.
    map_to_addr: Vaddr,
    /// The mapped memory object. This field specifies what type of memory is
    /// mapped and provides access to the underlying memory object (VMO, device
    /// memory, or anonymous memory).
    ///
    /// The start of the virtual address maps to the start of the range
    /// specified in the mapped object.
    mapped_mem: MappedMemory,
    /// The inode of the file that backs the mapping.
    ///
    /// If the inode is `Some`, it means that the mapping is file-backed.
    /// And the `mapped_mem` field must be the page cache of the inode, i.e.
    /// [`MappedMemory::Vmo`].
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
        mapped_mem: MappedMemory,
        inode: Option<Arc<dyn Inode>>,
        is_shared: bool,
        handle_page_faults_around: bool,
        perms: VmPerms,
    ) -> Self {
        Self {
            map_size,
            map_to_addr,
            mapped_mem,
            inode,
            is_shared,
            handle_page_faults_around,
            perms,
        }
    }

    pub(super) fn new_fork(&self) -> VmMapping {
        VmMapping {
            mapped_mem: self.mapped_mem.dup(),
            inode: self.inode.clone(),
            ..*self
        }
    }

    pub(super) fn clone_for_remap_at(&self, va: Vaddr) -> VmMapping {
        let mut vm_mapping = self.new_fork();
        vm_mapping.map_to_addr = va;
        vm_mapping
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

    /// Returns a reference to the VMO if this mapping is VMO-backed.
    pub(super) fn vmo(&self) -> Option<&MappedVmo> {
        match &self.mapped_mem {
            MappedMemory::Vmo(vmo) => Some(vmo),
            _ => None,
        }
    }

    /// Returns the mapping's RSS type.
    pub fn rss_type(&self) -> RssType {
        match &self.mapped_mem {
            MappedMemory::Anonymous => RssType::RSS_ANONPAGES,
            MappedMemory::Vmo(_) | MappedMemory::Device => RssType::RSS_FILEPAGES,
        }
    }

    /// Returns whether this mapping can be expanded.
    ///
    /// Device mappings cannot be expanded as they represent fixed-size MMIO
    /// regions.
    pub(super) fn can_expand(&self) -> bool {
        !matches!(self.mapped_mem, MappedMemory::Device)
    }

    /// Populates device memory for this mapping.
    ///
    /// This method should only be called for device memory mappings. It maps
    /// the provided I/O memory region into the virtual address space.
    ///
    /// # Panics
    ///
    /// In debug builds, this method panics if the mapping is not a device
    /// memory mapping.
    pub(super) fn populate_device(
        &self,
        vm_space: &VmSpace,
        io_mem: IoMem,
        vmo_offset: usize,
    ) -> Result<()> {
        debug_assert!(matches!(self.mapped_mem, MappedMemory::Device));

        let preempt_guard = disable_preempt();
        let map_range = self.map_to_addr..self.map_to_addr + self.map_size.get();
        let mut cursor = vm_space.cursor_mut(&preempt_guard, &map_range)?;
        let io_page_prop =
            PageProperty::new_user(PageFlags::from(self.perms), io_mem.cache_policy());
        cursor.map_iomem(io_mem, io_page_prop, self.map_size.get(), vmo_offset);

        Ok(())
    }

    /// Prints the mapping information in the format of `/proc/[pid]/maps`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/task_mmu.c#L304-L359>
    pub fn print_to_maps(&self, printer: &mut VmPrinter, name: &str) -> Result<()> {
        let start = self.map_to_addr;
        let end = self.map_end();
        let read_char = if self.perms.contains(VmPerms::READ) {
            'r'
        } else {
            '-'
        };
        let write_char = if self.perms.contains(VmPerms::WRITE) {
            'w'
        } else {
            '-'
        };
        let exec_char = if self.perms.contains(VmPerms::EXEC) {
            'x'
        } else {
            '-'
        };
        let shared_char = if self.is_shared { 's' } else { 'p' };
        let offset = self.vmo().map(|vmo| vmo.offset).unwrap_or(0);
        let (dev_major, dev_minor) = self
            .inode()
            .map(|inode| device_id::decode_device_numbers(inode.metadata().dev))
            .unwrap_or((0, 0));
        let ino = self.inode().map(|inode| inode.ino()).unwrap_or(0);

        writeln!(
            printer,
            "{:x}-{:x} {}{}{}{} {:08x} {:02x}:{:02x} {:<26} {}",
            start,
            end,
            read_char,
            write_char,
            exec_char,
            shared_char,
            offset,
            dev_major,
            dev_minor,
            ino,
            name
        )?;

        Ok(())
    }

    /// Returns whether this mapping is a COW mapping.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/include/linux/mm.h#L1470-L1473>
    fn is_cow(&self) -> bool {
        !self.is_shared && self.perms.contains(VmPerms::MAY_WRITE)
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
        self.check_perms_for_page_fault(page_fault_info)?;

        let page_aligned_addr = page_fault_info.address.align_down(PAGE_SIZE);
        let is_write = page_fault_info.required_perms.contains(VmPerms::WRITE);

        if !is_write
            && matches!(&self.mapped_mem, MappedMemory::Vmo(_))
            && self.handle_page_faults_around
        {
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
                if let (_, Some(_)) = cursor.query().unwrap() {
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

    fn check_perms_for_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        trace!(
            "self.perms {:?}, page_fault_info.required_perms {:?}, self.range {:?}",
            self.perms,
            page_fault_info.required_perms,
            self.range()
        );

        let mut perms = self.perms;

        // Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/mm/gup.c#L1282-L1311>
        if page_fault_info.is_forced {
            if perms.contains(VmPerms::MAY_READ) {
                perms.insert(VmPerms::READ);
            }
            if self.is_cow() {
                perms.insert(VmPerms::WRITE);
            }
        }

        if !perms.contains(page_fault_info.required_perms) {
            return_errno_with_message!(Errno::EACCES, "perm check fails");
        }

        Ok(())
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
                Some(VmQueriedItem::MappedRam { frame, mut prop }) => {
                    if VmPerms::from(prop.flags).contains(required_perms) {
                        // The page fault is already handled maybe by other threads.
                        // Just flush the TLB and return.
                        TlbFlushOp::for_range(va).perform_on_current();
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
                        cursor.flusher().issue_tlb_flush(TlbFlushOp::for_range(va));
                        cursor.flusher().dispatch_tlb_flush();
                    } else {
                        let new_frame = duplicate_frame(&frame)?;
                        prop.flags |= new_flags;
                        cursor.map(new_frame.into(), prop);
                    }
                    cursor.flusher().sync_tlb_flush();
                }
                Some(VmQueriedItem::MappedIoMem { .. }) => {
                    // The page of I/O memory is populated when the memory
                    // mapping is created.
                    return_errno_with_message!(
                        Errno::EFAULT,
                        "device memory page faults cannot be resolved"
                    );
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
                            self.vmo().unwrap().commit_on(index, CommitFlags::empty())?;
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

        let vmo = match &self.mapped_mem {
            MappedMemory::Vmo(vmo) => vmo,
            MappedMemory::Anonymous => {
                // Anonymous mapping. Allocate a new frame.
                return Ok((FrameAllocOptions::new().alloc_frame()?.into(), is_readonly));
            }
            MappedMemory::Device => {
                // Device memory is populated when the memory mapping is created.
                return Err(VmoCommitError::Err(Error::with_message(
                    Errno::EFAULT,
                    "device memory page faults cannot be resolved",
                )));
            }
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

        let vmo = self.vmo().unwrap();
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
                    start_addr = (index * PAGE_SIZE - vmo.offset()) + self.map_to_addr;
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
        debug_assert!(at.is_multiple_of(PAGE_SIZE));

        let (l_mapped_mem, r_mapped_mem) = match self.mapped_mem {
            MappedMemory::Vmo(vmo) => {
                let at_offset = vmo.offset() + (at - self.map_to_addr);
                let r_mapped_vmo = vmo.dup_at_offset(at_offset);
                (MappedMemory::Vmo(vmo), MappedMemory::Vmo(r_mapped_vmo))
            }
            MappedMemory::Anonymous => {
                // For anonymous mappings, we create new anonymous mappings for the split parts
                (MappedMemory::Anonymous, MappedMemory::Anonymous)
            }
            MappedMemory::Device => {
                // For device memory mappings, we create new device memory mappings for the split parts
                (MappedMemory::Device, MappedMemory::Device)
            }
        };

        let left_size = at - self.map_to_addr;
        let right_size = self.map_size.get() - left_size;
        let left = Self {
            map_to_addr: self.map_to_addr,
            map_size: NonZeroUsize::new(left_size).unwrap(),
            mapped_mem: l_mapped_mem,
            inode: self.inode.clone(),
            ..self
        };
        let right = Self {
            map_to_addr: at,
            map_size: NonZeroUsize::new(right_size).unwrap(),
            mapped_mem: r_mapped_mem,
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
        let mut new_flags = PageFlags::from(perms);
        if self.is_cow() && !self.perms.contains(VmPerms::WRITE) {
            new_flags.remove(PageFlags::W);
        }

        let preempt_guard = disable_preempt();
        let range = self.range();
        let mut cursor = vm_space.cursor_mut(&preempt_guard, &range).unwrap();

        let op = |flags: &mut PageFlags, _cache: &mut CachePolicy| *flags = new_flags;
        while cursor.virt_addr() < range.end {
            if let Some(va) = cursor.protect_next(range.end - cursor.virt_addr(), op) {
                cursor.flusher().issue_tlb_flush(TlbFlushOp::for_range(va));
            } else {
                break;
            }
        }
        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        Self { perms, ..self }
    }
}

/// Memory mapped by a [`VmMapping`].
#[derive(Debug)]
pub(super) enum MappedMemory {
    /// Anonymous memory.
    ///
    /// These pages are not associated with any files. On-demand population is possible by enabling
    /// page fault handlers to allocate zeroed pages.
    Anonymous,
    /// Memory in a [`Vmo`].
    ///
    /// These pages are associated with regular files that are backed by the page cache. On-demand
    /// population is possible by enabling page fault handlers to allocate pages and read the page
    /// content from the disk.
    Vmo(MappedVmo),
    /// Device memory.
    ///
    /// These pages are associated with special files (typically device memory). They are populated
    /// when the memory mapping is created via mmap, instead of occurring at page faults.
    Device,
}

impl MappedMemory {
    /// Duplicates the mapped memory capability.
    pub(super) fn dup(&self) -> Self {
        match self {
            MappedMemory::Anonymous => MappedMemory::Anonymous,
            MappedMemory::Vmo(v) => MappedMemory::Vmo(v.dup()),
            MappedMemory::Device => MappedMemory::Device,
        }
    }
}

/// A wrapper that represents a mapped [`Vmo`] and provide required functionalities
/// that need to be provided to mappings from the VMO.
#[derive(Debug)]
pub(super) struct MappedVmo {
    vmo: Arc<Vmo>,
    /// Represents the mapped offset in the VMO for the mapping.
    offset: usize,
    /// Whether the VMO's writable mappings need to be tracked, and the
    /// mapping is writable to the VMO.
    is_writable_tracked: bool,
}

impl MappedVmo {
    /// Creates a `MappedVmo` used for the mapping.
    pub(super) fn new(vmo: Arc<Vmo>, offset: usize, is_writable_tracked: bool) -> Result<Self> {
        if is_writable_tracked {
            vmo.writable_mapping_status().map()?;
        }

        Ok(Self {
            vmo,
            offset,
            is_writable_tracked,
        })
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
        debug_assert!(page_offset.is_multiple_of(PAGE_SIZE));
        self.vmo.try_commit_page(self.offset + page_offset)
    }

    /// Commits a page at a specific page index.
    ///
    /// This method may involve I/O operations if the VMO needs to fetch
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

    /// Gets a reference to the underlying VMO.
    pub fn vmo(&self) -> &Arc<Vmo> {
        &self.vmo
    }

    /// Gets the offset for the mappings.
    pub fn offset(&self) -> usize {
        self.offset
    }

    /// Duplicates the capability.
    pub fn dup(&self) -> Self {
        self.dup_at_offset(self.offset)
    }

    /// Duplicates the capability at a specific offset.
    fn dup_at_offset(&self, offset: usize) -> Self {
        if self.is_writable_tracked {
            self.vmo.writable_mapping_status().increment();
        }

        Self {
            vmo: self.vmo.clone(),
            offset,
            is_writable_tracked: self.is_writable_tracked,
        }
    }
}

impl Drop for MappedVmo {
    fn drop(&mut self) {
        if self.is_writable_tracked {
            self.vmo.writable_mapping_status().decrement();
        }
    }
}

/// Attempts to merge two [`VmMapping`]s into a single mapping if they are
/// adjacent and compatible.
///
/// - Returns the merged [`VmMapping`] if successful. The caller should
///   remove the original mappings before inserting the merged mapping
///   into the [`Vmar`].
/// - Returns `None` otherwise.
///
/// [`Vmar`]: crate::vm::vmar::Vmar
fn try_merge(left: &VmMapping, right: &VmMapping) -> Option<VmMapping> {
    let is_adjacent = left.map_end() == right.map_to_addr();
    let is_type_equal = left.is_shared == right.is_shared
        && left.handle_page_faults_around == right.handle_page_faults_around
        && left.perms == right.perms;

    if !is_adjacent || !is_type_equal {
        return None;
    }

    let mapped_mem = match (&left.mapped_mem, &right.mapped_mem) {
        (MappedMemory::Anonymous, MappedMemory::Anonymous) => {
            // Anonymous memory mappings can be merged if they are adjacent
            MappedMemory::Anonymous
        }
        (MappedMemory::Vmo(l_vmo_obj), MappedMemory::Vmo(r_vmo_obj)) => {
            let l_vmo = l_vmo_obj.vmo();
            let r_vmo = r_vmo_obj.vmo();

            if Arc::ptr_eq(l_vmo, r_vmo) {
                let is_offset_contiguous =
                    l_vmo_obj.offset() + left.map_size() == r_vmo_obj.offset();
                if !is_offset_contiguous {
                    return None;
                }
                MappedMemory::Vmo(l_vmo_obj.dup())
            } else {
                return None;
            }
        }
        // Device memory and other types cannot be merged
        _ => return None,
    };

    let map_size = NonZeroUsize::new(left.map_size() + right.map_size()).unwrap();

    Some(VmMapping {
        map_size,
        mapped_mem,
        inode: left.inode.clone(),
        ..*left
    })
}

fn duplicate_frame(src: &UFrame) -> Result<Frame<()>> {
    let new_frame = FrameAllocOptions::new().zeroed(false).alloc_frame()?;
    new_frame.writer().write(&mut src.reader());
    Ok(new_frame)
}
