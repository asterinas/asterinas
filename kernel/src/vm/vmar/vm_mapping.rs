// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::Cow, format};
use core::{num::NonZeroUsize, ops::Range};

use align_ext::AlignExt;
use aster_util::printer::VmPrinter;
use ostd::{
    io::IoMem,
    mm::{
        CachePolicy, HIGHEST_PAGING_LEVEL, PageFlags, PagingLevel, UFrame, page_size_at,
        tlb::TlbFlushOp, vm_space::CursorMut,
    },
};

use super::{
    RssType,
    interval_set::Interval,
    util::is_intersected,
    vmar_impls::{PerPtMeta, RsAsDelta},
};
use crate::{
    fs::{
        path::{Path, PathResolver},
        utils::Inode,
    },
    prelude::*,
    vm::{
        perms::VmPerms,
        vmar::Vmar,
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
/// This type controls the actual mapping in the [`VmarSpace`]. It is a linear
/// type and cannot be [`Drop`]. To remove a mapping, use [`Self::unmap`].
///
/// [`Vmar`]: crate::vm::vmar::Vmar
/// [`VmarSpace`]: crate::vm::vmar::VmarSpace
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
    /// Number of base frames mapped in the page table for this mapping.
    ///
    /// If it is [`None`], we need to walk the page table to count the mapped
    /// pages. It can happen when [`VmMapping::split`] is called.
    frames_mapped: Option<usize>,
    /// The inode of the file that backs the mapping.
    ///
    /// If the inode is `Some`, it means that the mapping is file-backed.
    /// And the `mapped_mem` field must be the page cache of the inode, i.e.
    /// [`MappedMemory::Vmo`].
    inode: Option<Arc<dyn Inode>>,
    /// The path of the file that backs the mapping.
    path: Option<Path>,
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
    #[expect(clippy::too_many_arguments)]
    pub(super) fn new(
        map_size: NonZeroUsize,
        map_to_addr: Vaddr,
        mapped_mem: MappedMemory,
        inode: Option<Arc<dyn Inode>>,
        path: Option<Path>,
        is_shared: bool,
        handle_page_faults_around: bool,
        perms: VmPerms,
    ) -> Self {
        Self {
            map_size,
            map_to_addr,
            mapped_mem,
            frames_mapped: Some(0),
            inode,
            path,
            is_shared,
            handle_page_faults_around,
            perms,
        }
    }

    pub(super) fn new_fork(&self) -> VmMapping {
        VmMapping {
            mapped_mem: self.mapped_mem.dup(),
            inode: self.inode.clone(),
            path: self.path.clone(),
            ..*self
        }
    }

    pub(super) fn frames_mapped(&self) -> Option<usize> {
        self.frames_mapped
    }

    pub(super) fn inc_frames_mapped(&mut self) {
        if let Some(frames_mapped) = self.frames_mapped.as_mut() {
            *frames_mapped += 1;

            debug_assert!(*frames_mapped <= self.map_size() / PAGE_SIZE);
        }
    }

    pub(super) fn dec_frames_mapped(&mut self, num_frames: usize) {
        if let Some(frames_mapped) = self.frames_mapped.as_mut() {
            *frames_mapped -= num_frames;
        }
    }

    pub(super) fn set_fully_mapped(&mut self) {
        self.frames_mapped = Some(self.map_size() / PAGE_SIZE);
    }

    pub(super) fn remap_at(mut self, va: Vaddr) -> VmMapping {
        self.map_to_addr = va;
        self
    }

    pub fn clone_for_check(&self) -> VmMapping {
        self.new_fork()
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
            MappedMemory::Vmo(_) | MappedMemory::Device(_) => RssType::RSS_FILEPAGES,
        }
    }

    /// Returns whether this mapping can be expanded.
    ///
    /// Device mappings cannot be expanded as they represent fixed-size MMIO
    /// regions.
    pub(super) fn can_expand(&self) -> bool {
        !matches!(self.mapped_mem, MappedMemory::Device(_))
    }

    /// Prints the mapping information in the format of `/proc/[pid]/maps`.
    ///
    /// Reference: <https://elixir.bootlin.com/linux/v6.16.5/source/fs/proc/task_mmu.c#L304-L359>
    pub fn print_to_maps(
        &self,
        printer: &mut VmPrinter,
        parent_vmar: &Vmar,
        path_resolver: &PathResolver,
    ) -> Result<()> {
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

        let line = format!(
            "{:x}-{:x} {}{}{}{} {:08x} {:02x}:{:02x} {} ",
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
        );

        let name = || {
            let process_vm = parent_vmar.process_vm();
            let user_stack_top = process_vm.init_stack().user_stack_top();

            if self.map_to_addr <= user_stack_top && self.map_end() > user_stack_top {
                return Some(Cow::Borrowed("[stack]"));
            }

            let heap_range = process_vm.heap().heap_range();
            if self.map_to_addr >= heap_range.start && self.map_end() <= heap_range.end {
                return Some(Cow::Borrowed("[heap]"));
            }

            #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
            if let Some(vmo) = self.vmo() {
                use crate::vdso::{VDSO_VMO_LAYOUT, vdso_vmo};

                if let Some(vdso_vmo) = vdso_vmo()
                    && Arc::ptr_eq(&vmo.vmo, &vdso_vmo)
                {
                    let offset = vmo.offset();
                    if offset == VDSO_VMO_LAYOUT.data_segment_offset {
                        return Some(Cow::Borrowed("[vvar]"));
                    } else if offset == VDSO_VMO_LAYOUT.text_segment_offset {
                        return Some(Cow::Borrowed("[vdso]"));
                    }
                }
            }

            if let Some(path) = &self.path {
                return Some(Cow::Owned(path_resolver.make_abs_path(path).into_string()));
            }

            // Reference: <https://github.com/google/gvisor/blob/38123b53da96ff6983fcc103dfe2a9cc4e0d80c8/test/syscalls/linux/proc.cc#L1158-L1172>
            if matches!(&self.mapped_mem, MappedMemory::Vmo(_)) && self.is_shared {
                return Some(Cow::Borrowed("/dev/zero (deleted)"));
            }

            // Common anonymous mappings do not have names.
            None
        };

        let name = name();

        if let Some(name) = name {
            writeln!(printer, "{:<72} {}", line, name)?;
        } else {
            writeln!(printer, "{}", line)?;
        }

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
            MappedMemory::Anonymous => (MappedMemory::Anonymous, MappedMemory::Anonymous),
            MappedMemory::Device(_io_mem) => {
                return_errno_with_message!(Errno::EINVAL, "splitting device mapping unsupported")
            }
        };

        let left_size = at - self.map_to_addr;
        let right_size = self.map_size.get() - left_size;

        let (left_frames_mapped, right_frames_mapped) =
            if let Some(frames_mapped) = self.frames_mapped {
                if frames_mapped == 0 {
                    (Some(0), Some(0))
                } else if frames_mapped == self.map_size.get() / PAGE_SIZE {
                    (Some(left_size / PAGE_SIZE), Some(right_size / PAGE_SIZE))
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
            frames_mapped: left_frames_mapped,
            inode: self.inode.clone(),
            path: self.path.clone(),
            ..self
        };
        let right = Self {
            map_to_addr: at,
            map_size: NonZeroUsize::new(right_size).unwrap(),
            mapped_mem: r_mapped_mem,
            frames_mapped: right_frames_mapped,
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

    /// Splits the mapping into multiple mappings at page table boundaries.
    ///
    /// When split, each mapping can be mapped using a single page table at a
    /// level lower than the specified `max_level`. And the boundaries of the
    /// mappings will be aligned to the page size of the level.
    pub(super) fn split_for_pt(
        self,
        max_level: PagingLevel,
    ) -> impl Iterator<Item = (Self, PagingLevel)> {
        debug_assert!(self.map_to_addr.is_multiple_of(page_size_at(1)));
        debug_assert!(1 <= max_level);
        debug_assert!(max_level <= HIGHEST_PAGING_LEVEL);

        fn next_pt_boundary(va: Vaddr, level: PagingLevel) -> Option<Vaddr> {
            va.checked_add(1)?.checked_align_up(page_size_at(level + 1))
        }

        let mut remaining = Some(self);

        core::iter::from_fn(move || {
            let mapping = remaining.take()?;

            let range = mapping.map_to_addr()..mapping.map_end();

            let mut level = max_level;

            let break_at = loop {
                if let Some(pt_end) = next_pt_boundary(range.start, level)
                    && range.start.is_multiple_of(page_size_at(level))
                {
                    if pt_end <= range.end {
                        break pt_end;
                    } else {
                        let end_aligned = range.end.align_down(page_size_at(level));
                        if end_aligned > range.start {
                            break end_aligned;
                        }
                    }
                }

                level -= 1;
            };

            let res_mapping = if break_at < range.end {
                let (a, b) = mapping.split(break_at).unwrap();
                remaining = Some(b);
                a
            } else {
                mapping
            };

            Some((res_mapping, level))
        })
    }

    /// Returns whether this mapping can be merged with the given `vm_mapping`.
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
    pub fn can_merge_with(&self, vm_mapping: &VmMapping) -> bool {
        debug_assert!(!is_intersected(&self.range(), &vm_mapping.range()));

        let (left, right) = if self.map_to_addr < vm_mapping.map_to_addr {
            (self, vm_mapping)
        } else {
            (vm_mapping, self)
        };

        can_merge(left, right)
    }

    /// Attempts to merge `self` with the given `vm_mapping`.
    ///
    /// Two mappings can be merged if they are adjacent and compatible. See
    /// [`Self::can_merge_with`] for details.
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
        let level = cursor.level();

        let op = |flags: &mut PageFlags, _cache: &mut CachePolicy| *flags = new_flags;

        while cursor.find_next(range.end - cursor.virt_addr()).is_some() {
            debug_assert_eq!(cursor.level(), level);
            cursor.protect(op);

            let va = cursor.cur_va_range();

            cursor
                .flusher()
                .issue_tlb_flush(TlbFlushOp::for_range(va.clone()));
            debug_assert!(va.end <= range.end);
            if cursor.jump(va.end).is_err() {
                break;
            }
        }

        Self { perms, ..self }
    }

    /// Unmaps the mapping.
    pub(super) fn unmap(self, cursor: &mut CursorMut<PerPtMeta>, rs_as_delta: &mut RsAsDelta) {
        let range = self.range();
        cursor.jump(range.start).unwrap();
        let level = cursor.level();

        let mut num_unmapped_pages = 0;

        while cursor.find_next(range.end - cursor.virt_addr()).is_some() {
            debug_assert_eq!(cursor.level(), level);
            let unmapped_pages = cursor.unmap();
            num_unmapped_pages += unmapped_pages;

            let va = cursor.cur_va_range();

            cursor
                .flusher()
                .issue_tlb_flush(TlbFlushOp::for_range(va.clone()));
            debug_assert!(va.end <= range.end);
            if cursor.jump(va.end).is_err() {
                break;
            }
        }

        debug_assert_eq!(
            num_unmapped_pages,
            self.frames_mapped.unwrap_or(num_unmapped_pages)
        );
        rs_as_delta.add_rs(self.rss_type(), -(num_unmapped_pages as isize));
        rs_as_delta.sub_as(range.len());
    }
}

/// Memory mapped by a [`VmMapping`].
#[derive(Debug)]
pub(super) enum MappedMemory {
    /// Anonymous memory.
    ///
    /// These pages are not associated with any files. On-demand population is
    /// possible by enabling page fault handlers to allocate zeroed pages.
    Anonymous,
    /// Memory in a [`Vmo`].
    ///
    /// These pages are associated with regular files that are backed by the
    /// page cache. On-demand population is possible by enabling page fault
    /// handlers to allocate pages and read the page content from the disk.
    Vmo(MappedVmo),
    /// Device memory.
    ///
    /// These pages are associated with special device memory files. They are
    /// populated when the memory mapping is created via mmap. And the provided
    /// `usize` is the offset in the device memory.
    Device(IoMem),
}

impl MappedMemory {
    /// Duplicates the mapped memory capability.
    pub(super) fn dup(&self) -> Self {
        match self {
            MappedMemory::Anonymous => MappedMemory::Anonymous,
            MappedMemory::Vmo(v) => MappedMemory::Vmo(v.dup()),
            MappedMemory::Device(io_mem) => MappedMemory::Device(io_mem.clone()),
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
    ///
    /// If the offset is beyond the VMO size, returns `None`.
    pub(super) fn valid_size(&self) -> Option<usize> {
        self.vmo.size().checked_sub(self.offset)
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
    // FIXME: The `page_idx` is not relative to the mapping offset but the
    // underlying VMO. Consider encapsulating the inner implementation.
    pub fn dup_commit(&self, page_idx: usize, commit_flags: CommitFlags) -> VmoCommitHandle {
        VmoCommitHandle {
            vmo: self.vmo.clone(),
            page_idx,
            commit_flags,
        }
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
#[derive(Debug)]
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

/// Returns whether two [`VmMapping`]s can be merged.
fn can_merge(left: &VmMapping, right: &VmMapping) -> bool {
    let is_adjacent = left.map_end() == right.map_to_addr();
    let is_type_equal = left.is_shared == right.is_shared
        && left.handle_page_faults_around == right.handle_page_faults_around
        && left.perms == right.perms;

    if !is_adjacent || !is_type_equal {
        return false;
    }

    match (&left.mapped_mem, &right.mapped_mem) {
        (MappedMemory::Anonymous, MappedMemory::Anonymous) => true,
        (MappedMemory::Vmo(l_vmo_obj), MappedMemory::Vmo(r_vmo_obj)) => {
            if l_vmo_obj.is_same_vmo(r_vmo_obj) {
                l_vmo_obj.offset() + left.map_size() == r_vmo_obj.offset()
            } else {
                false
            }
        }
        (MappedMemory::Device(_), MappedMemory::Device(_)) => false,
        _ => false,
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
    if !can_merge(left, right) {
        return None;
    }

    let mapped_mem = match (&left.mapped_mem, &right.mapped_mem) {
        (MappedMemory::Anonymous, MappedMemory::Anonymous) => MappedMemory::Anonymous,
        (MappedMemory::Vmo(l_vmo_obj), MappedMemory::Vmo(_r_vmo_obj)) => {
            MappedMemory::Vmo(l_vmo_obj.dup())
        }
        (MappedMemory::Device(l_io_mem), MappedMemory::Device(_r_io_mem)) => {
            MappedMemory::Device(l_io_mem.clone())
        }
        _ => return None,
    };

    let map_size = NonZeroUsize::new(left.map_size() + right.map_size()).unwrap();

    // Combine the accounted mapped frames if both sides tracked them; otherwise
    // fall back to `None` so callers will recount on demand.
    let frames_mapped = match (left.frames_mapped, right.frames_mapped) {
        (Some(l), Some(r)) => Some(l + r),
        _ => None,
    };

    Some(VmMapping {
        map_size,
        mapped_mem,
        inode: left.inode.clone(),
        path: left.path.clone(),
        frames_mapped,
        ..*left
    })
}
