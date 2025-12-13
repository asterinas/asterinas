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

use super::{RssDelta, RssType, interval_set::Interval};
use crate::{
    fs::utils::Inode,
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::{
        perms::VmPerms,
        vmar::is_intersected,
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
    /// Number of bytes currently mapped in the page table for this mapping.
    ///
    /// If it is [`None`], we need to walk the page table to count the mapped
    /// pages. It can happen when [`VmMapping::split`] is called.
    bytes_mapped: Option<usize>,
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
            bytes_mapped: Some(0),
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

    pub(super) fn bytes_mapped(&self) -> Option<usize> {
        self.bytes_mapped.clone()
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

    /// Returns whether to handle surrounding pages when handling page fault.
    pub fn handle_page_faults_around(&self) -> bool {
        self.handle_page_faults_around
    }

    pub(super) fn mapped_mem(&self) -> &MappedMemory {
        &self.mapped_mem
    }

    pub fn is_shared(&self) -> bool {
        self.is_shared
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

        let (left_bytes_mapped, right_bytes_mapped) = if let Some(bytes_mapped) = self.bytes_mapped
        {
            if bytes_mapped == 0 {
                (Some(0), Some(0))
            } else if bytes_mapped == self.map_size {
                (Some(left_size), Some(right_size))
            } else {
                (None, None)
            }
        } else {
            (None, None)
        };

        let left = Self {
            map_to_addr: self.map_to_addr,
            map_size: NonZeroUsize::new(left_size).unwrap(),
            mapped_mem: l_mapped_mem,
            bytes_mapped: left_bytes_mapped,
            inode: self.inode.clone(),
            ..self
        };
        let right = Self {
            map_to_addr: at,
            map_size: NonZeroUsize::new(right_size).unwrap(),
            mapped_mem: r_mapped_mem,
            bytes_mapped: right_bytes_mapped,
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

    /// Change the perms of the mapping.
    pub(super) fn protect(self, cursor: &mut CursorMut<PerPtMeta>, perms: VmPerms) -> Self {
        let mut new_flags = PageFlags::from(perms);
        if self.is_cow() && !self.perms.contains(VmPerms::WRITE) {
            new_flags.remove(PageFlags::W);
        }

        let range = self.range();
        cursor.jump(range.start).unwrap();

        let op = |flags: &mut PageFlags, _cache: &mut CachePolicy| *flags = new_flags;

        while cursor.find_next(range.end - cursor.virt_addr()).is_some() {
            cursor.protect(op);

            cursor
                .flusher()
                .issue_tlb_flush(TlbFlushOp::for_range(va.clone()));

            let va = cursor.cur_va_range();
            debug_assert!(va.end <= range.end);
            if va.end == range.end {
                break;
            }
        }

        Self { perms, ..self }
    }

    /// Unmaps the mapping.
    pub(super) fn unmap(self, cursor: &mut CursorMut<PerPtMeta>, rss_delta: &mut RssDelta) {
        let range = self.range();
        cursor.jump(range.start).unwrap();

        while cursor
            .find_next_unmappable_subtree(range.end - cursor.virt_addr())
            .is_some()
        {
            let unmapped_pages = cursor.unmap();
            rss_delta.add(self.rss_type(), -(unmapped_pages as isize));

            cursor
                .flusher()
                .issue_tlb_flush(TlbFlushOp::for_range(va.clone()));

            let va = cursor.cur_va_range();
            debug_assert!(va.end <= range.end);
            if va.end == range.end {
                break;
            }
        }
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
    pub(super) fn valid_size(&self) -> usize {
        let vmo_size = self.vmo.size();
        (self.offset..vmo_size).len()
    }

    /// Gets the committed frame at the input offset in the mapped VMO.
    ///
    /// If the VMO has not committed a frame at this index, it will commit
    /// one first and return it. If the commit operation needs to perform I/O,
    /// it will return a [`VmoCommitError::NeedIo`].
    pub(super) fn get_committed_frame(
        &self,
        page_offset: usize,
    ) -> core::result::Result<UFrame, VmoCommitError> {
        debug_assert!(page_offset.is_multiple_of(PAGE_SIZE));
        self.vmo.try_commit_page(self.offset + page_offset)
    }

    /// Gets a [`VmoCommitHandle`].
    pub fn dup_commit(&self, page_idx: usize, commit_flags: CommitFlags) -> VmoCommitHandle {
        VmoCommitHandle {
            vmo: self.vmo.clone(),
            page_idx: self.offset + page_idx,
            commit_flags,
        }
    }

    /// Tries to commit a page at the given offset in the mapped VMO.
    ///
    /// Unlike [`VmoCommitHandle::commit`], this method will return error if
    /// I/O is required to commit the page.
    pub fn try_commit_page(
        &self,
        page_offset: usize,
        commit_flags: CommitFlags,
    ) -> core::result::Result<UFrame, VmoCommitError> {
        debug_assert!(page_offset.is_multiple_of(PAGE_SIZE));
        self.vmo
            .try_commit_page_on(page_offset + self.offset, commit_flags)
    }

    /// Returns if the two [`MappedVmo`]s refer to the same underlying [`Vmo`].
    pub fn is_same_vmo(&self, other: &MappedVmo) -> bool {
        Arc::ptr_eq(&self.vmo, &other.vmo)
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

/// A handle to [`MappedVmo`] that can only commit pages.
pub(super) struct VmoCommitHandle {
    vmo: Arc<Vmo>,
    page_idx: usize,
    commit_flags: CommitFlags,
}

impl VmoCommitHandle {
    pub fn commit(self) -> Result<UFrame> {
        self.vmo.commit_on(self.page_idx, self.commit_flags)
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
/// [`Vmar`]: super::Vmar
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
            if l_vmo_obj.is_same_vmo(r_vmo_obj) {
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
