// SPDX-License-Identifier: MPL-2.0

use core::{num::NonZeroUsize, ops::Range};

use ostd::{
    mm::{CachePolicy, FrameAllocOptions, PageFlags, PageProperty},
    task::{DisabledPreemptGuard, disable_preempt},
};

use super::{MappedMemory, MappedVmo, RsAsDelta, VmMapping, Vmar};
use crate::{
    fs::{
        file_handle::{FileLike, Mappable},
        path::Path,
        ramfs::memfd::MemfdInode,
    },
    prelude::*,
    vm::{
        perms::VmPerms,
        vmar::{
            cursor_util::find_next_mapped,
            interval_set::Interval,
            is_userspace_vaddr_range,
            util::is_intersected,
            vm_allocator::AllocatorGuard,
            vmar_impls::{PteRangeMeta, VmarCursorMut},
        },
        vmo::{CommitFlags, Vmo},
    },
};

impl Vmar {
    /// Creates a mapping into the VMAR through a set of VMAR mapping options.
    ///
    /// # Examples
    ///
    /// ```
    /// use ostd::mm::PAGE_SIZE;
    ///
    /// use crate::vm::{perms::VmPerms, vmar::Vmar, vmo::VmoOptions};
    ///
    /// let vmar = Vmar::new();
    /// let vmo = VmoOptions::new(10 * PAGE_SIZE).alloc().unwrap();
    /// let target_vaddr = 0x1234000;
    /// let real_vaddr = vmar
    ///     // Create a 4 * PAGE_SIZE bytes, read-only mapping
    ///     .new_map(PAGE_SIZE * 4, VmPerms::READ).unwrap()
    ///     // Provide an optional offset for the mapping inside the VMAR
    ///     .offset(target_vaddr)
    ///     // Specify an optional binding VMO.
    ///     .vmo(vmo)
    ///     // Provide an optional offset to indicate the corresponding offset
    ///     // in the VMO for the mapping
    ///     .vmo_offset(2 * PAGE_SIZE)
    ///     .build()
    ///     .unwrap();
    /// assert!(real_vaddr == target_vaddr);
    /// ```
    ///
    /// For more details on the available options, see `VmarMapOptions`.
    pub fn new_map(&self, size: NonZeroUsize, perms: VmPerms) -> Result<VmarMapOptions<'_>> {
        Ok(VmarMapOptions::new(self, size, perms))
    }

    /// Reserves a range to exclude it from future allocations.
    ///
    /// If the function succeeds, the range will not be allocated for future
    /// allocations. There's two ways to reclaim reserved regions:
    ///  - [`Self::new_map`] without both [`VmarMapOptions::offset`] and
    ///    [`OffsetType::Fixed`];
    ///  - [`Self::remap`].
    ///
    /// The function returns the starting virtual address of the reserved
    /// range. And it returns [`Errno::ENOMEM`] there's not enough free space
    /// to reserve.
    pub fn reserve(&self, size: NonZeroUsize, align: usize) -> Result<Vaddr> {
        assert!(align.is_power_of_two() && align.is_multiple_of(PAGE_SIZE));
        self.new_map(size, VmPerms::empty())?.align(align).build()
    }

    /// Reserves a specific range to exclude it from future allocations.
    ///
    /// See [`Self::reserve`] for details.
    ///
    /// The function returns [`Errno::ENOMEM`] if the range is already reserved
    /// or allocated.
    pub fn reserve_specific(&self, range: Range<Vaddr>) -> Result<()> {
        self.new_map(
            NonZeroUsize::new(range.end - range.start).unwrap(),
            VmPerms::empty(),
        )?
        .offset(range.start, OffsetType::FixedNoReplace)
        .build()
        .map(|_| ())
    }
}

/// Options for creating a new mapping. The mapping is not allowed to overlap
/// with any child VMARs. And unless specified otherwise, it is not allowed
/// to overlap with any existing mapping, either.
pub struct VmarMapOptions<'a> {
    parent: &'a Vmar,
    vmo: Option<Arc<Vmo>>,
    mappable: Option<Mappable>,
    path: Option<Path>,
    perms: VmPerms,
    may_perms: VmPerms,
    vmo_offset: usize,
    size: NonZeroUsize,
    offset: Option<(usize, OffsetType)>,
    align: usize,
    // Whether the mapping is mapped with `MAP_SHARED`
    is_shared: bool,
    // Whether the mapping needs to handle surrounding pages when handling page fault.
    handle_page_faults_around: bool,
    // Whether to map all pages immediately instead of on-demand.
    populate: bool,
}

impl<'a> VmarMapOptions<'a> {
    /// Creates a default set of options with the size and the memory access
    /// permissions.
    pub fn new(parent: &'a Vmar, size: NonZeroUsize, perms: VmPerms) -> Self {
        Self {
            parent,
            vmo: None,
            mappable: None,
            path: None,
            perms,
            may_perms: VmPerms::ALL_MAY_PERMS,
            vmo_offset: 0,
            size,
            offset: None,
            align: PAGE_SIZE,
            is_shared: false,
            handle_page_faults_around: false,
            populate: false,
        }
    }

    /// Sets the `VmPerms::MAY*` memory access permissions of the mapping.
    ///
    /// The default value is `MAY_READ | MAY_WRITE | MAY_EXEC`.
    ///
    /// The provided `may_perms` must be a subset of all the may-permissions,
    /// and must include the may-permissions corresponding to already requested
    /// normal permissions (`READ | WRITE | EXEC`).
    pub fn may_perms(mut self, may_perms: VmPerms) -> Self {
        self.may_perms = may_perms;
        self
    }

    /// Binds a [`Vmo`] to the mapping.
    ///
    /// If the mapping is a private mapping, its size may not be equal to that
    /// of the [`Vmo`]. For example, it is OK to create a mapping whose size is
    /// larger than that of the [`Vmo`], although one cannot read from or write
    /// to the part of the mapping that is not backed by the [`Vmo`].
    ///
    /// Such _oversized_ mappings are useful for two reasons:
    ///  1. [`Vmo`]s are resizable. So even if a mapping is backed by a VMO
    ///     whose size is equal to that of the mapping initially, we cannot
    ///     prevent the VMO from shrinking.
    ///  2. Mappings are not allowed to overlap by default. As a result,
    ///     oversized mappings can reserve space for future expansions.
    ///
    /// The [`Vmo`] of a mapping will be implicitly set if [`Self::mappable`] is
    /// set with a [`Mappable::Inode`].
    ///
    /// # Panics
    ///
    /// This function panics if a [`Mappable`] is already provided.
    pub fn vmo(mut self, vmo: Arc<Vmo>) -> Self {
        if self.mappable.is_some() {
            panic!("Cannot set `vmo` when `mappable` is already set");
        }
        self.vmo = Some(vmo);

        self
    }

    /// Sets the [`Path`] of the mapping.
    ///
    /// # Panics
    ///
    /// This function panics if a [`Mappable`] is already provided.
    pub fn path(mut self, path: Path) -> Self {
        if self.mappable.is_some() {
            panic!("Cannot set `path` when `mappable` is already set");
        }
        self.path = Some(path);
        self
    }

    /// Sets the offset of the first memory page in the VMO that is to be
    /// mapped into the VMAR.
    ///
    /// The offset must be page-aligned and within the VMO.
    ///
    /// The default value is zero.
    pub fn vmo_offset(mut self, offset: usize) -> Self {
        self.vmo_offset = offset;
        self
    }

    /// Sets the mapping's alignment.
    ///
    /// The default value is the page size.
    ///
    /// The provided alignment must be a power of two and a multiple of the
    /// page size.
    pub fn align(mut self, align: usize) -> Self {
        self.align = align;
        self
    }

    /// Sets the mapping's offset inside the VMAR.
    ///
    /// The offset must satisfy the alignment requirement.
    /// Also, the mapping's range `[offset, offset + size)` must be within
    /// the VMAR.
    ///
    /// If not set, the system will choose an offset automatically.
    pub fn offset(mut self, offset: usize, typ: OffsetType) -> Self {
        self.offset = Some((offset, typ));
        self
    }

    /// Sets whether the mapping can be shared with other process.
    ///
    /// The default value is false.
    ///
    /// If this value is set to true, the mapping will be shared with child
    /// process when forking.
    #[expect(clippy::wrong_self_convention)]
    pub fn is_shared(mut self, is_shared: bool) -> Self {
        self.is_shared = is_shared;
        self
    }

    /// Sets the mapping to handle surrounding pages when handling page fault.
    pub fn handle_page_faults_around(mut self) -> Self {
        self.handle_page_faults_around = true;
        self
    }

    /// Binds memory to map based on the [`Mappable`] enum.
    ///
    /// This method accepts file-specific details, like a page cache (inode)
    /// or I/O memory, but not both simultaneously.
    ///
    /// # Panics
    ///
    /// This function panics if a [`Vmo`], [`Path`] or [`Mappable`] is already provided.
    ///
    /// # Errors
    ///
    /// This function returns an error if the file does not have a corresponding
    /// mappable object of [`crate::fs::file_handle::Mappable`].
    pub fn mappable(mut self, file: &dyn FileLike) -> Result<Self> {
        if self.vmo.is_some() {
            panic!("Cannot set `mappable` when `vmo` is already set");
        }
        if self.path.is_some() {
            panic!("Cannot set `mappable` when `path` is already set");
        }
        if self.mappable.is_some() {
            panic!("Cannot set `mappable` when `mappable` is already set");
        }

        let mappable = file.mappable()?;

        // Verify whether the page cache inode is valid.
        if let Mappable::Inode(ref inode) = mappable {
            self.vmo = Some(inode.page_cache().expect("Map an inode without page cache"));
        }

        self.mappable = Some(mappable);
        self.path = Some(file.path().clone());

        Ok(self)
    }

    /// Sets whether to populate all pages immediately instead of on-demand.
    pub fn populate(mut self) -> Self {
        self.populate = true;
        self
    }

    /// Creates the mapping and adds it to the parent VMAR.
    ///
    /// All options will be checked at this point.
    ///
    /// On success, the virtual address of the new mapping is returned.
    pub fn build(self) -> Result<Vaddr> {
        self.check_options()?;
        let Self {
            parent,
            vmo,
            mappable,
            path,
            perms,
            may_perms,
            vmo_offset,
            size: map_size,
            offset,
            align,
            is_shared,
            handle_page_faults_around,
            mut populate,
        } = self;

        if populate {
            readahead_for_populate(vmo.clone(), vmo_offset, map_size)?;
        }

        let preempt_guard = disable_preempt();

        let (map_to_addr, _alloc_guard, mut cursor) =
            allocate_range(&preempt_guard, parent, offset, align, map_size)?;

        debug!(
            "map_size = {:#x}, offset = {:x?}, align = {:#x}; allocated to {:#x}",
            map_size, offset, align, map_to_addr
        );

        if matches!(mappable, Some(Mappable::IoMem(_))) {
            populate = true;
        }

        let vm_mapping = build_vm_mapping(
            mappable,
            path,
            vmo,
            vmo_offset,
            map_size,
            map_to_addr,
            perms,
            may_perms,
            is_shared,
            handle_page_faults_around,
        )?;

        parent.add_mapping_size(&preempt_guard, map_size.get())?;

        if populate {
            map_populate(&mut cursor, vm_mapping);
        } else {
            map_to_page_table(&mut cursor, vm_mapping);
        }

        Ok(map_to_addr)
    }

    /// Checks whether all options are valid.
    fn check_options(&self) -> Result<()> {
        debug_assert!(self.align.is_multiple_of(PAGE_SIZE));
        debug_assert!(self.align.is_power_of_two());
        debug_assert!(self.size.get().is_multiple_of(self.align));
        debug_assert!(self.vmo_offset.is_multiple_of(self.align));

        if let Some((offset, _)) = self.offset {
            debug_assert!(offset.is_multiple_of(self.align));

            if !is_userspace_vaddr_range(offset, self.size.get()) {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the specified offset and size exceed userspace address range"
                );
            }
        }

        self.check_perms()
    }

    /// Checks whether the permissions of the mapping is valid.
    fn check_perms(&self) -> Result<()> {
        if !VmPerms::ALL_MAY_PERMS.contains(self.may_perms)
            || !VmPerms::ALL_PERMS.contains(self.perms)
        {
            return_errno_with_message!(Errno::EACCES, "invalid may perms");
        }

        #[cfg(any(target_arch = "x86_64", target_arch = "aarch64"))]
        {
            // On x86_64 and aarch64, WRITE permission implies READ permission.
            if self.perms.contains(VmPerms::WRITE) && !self.perms.contains(VmPerms::READ) {
                return_errno_with_message!(Errno::EACCES, "missing read permission");
            }
            if self.may_perms.contains(VmPerms::MAY_WRITE)
                && !self.may_perms.contains(VmPerms::MAY_READ)
            {
                return_errno_with_message!(Errno::EACCES, "missing may read permission");
            }
        }

        let vm_perms = self.perms | self.may_perms;
        vm_perms.check()
    }
}

/// The type of offset specified in [`VmarMapOptions::offset`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum OffsetType {
    Hint,
    Fixed,
    FixedNoReplace,
}

fn allocate_range<'a>(
    preempt_guard: &'a DisabledPreemptGuard,
    parent: &'a Vmar,
    offset: Option<(usize, OffsetType)>,
    align: usize,
    map_size: NonZeroUsize,
) -> Result<(Vaddr, AllocatorGuard<'a>, VmarCursorMut<'a>)> {
    let map_size_bytes = map_size.get();

    #[cfg_attr(not(debug_assertions), expect(unused_mut))]
    let (map_to_addr, alloc_guard, mut cursor) = match offset {
        None => parent.allocator.alloc_and_lock(
            preempt_guard,
            parent.vm_space(),
            map_size_bytes,
            align,
        )?,
        Some((offset, OffsetType::Fixed)) => {
            let range = offset..offset + map_size_bytes;

            let mut rs_as_delta = RsAsDelta::new(parent);

            let (alloc_guard, mut cursor) =
                parent
                    .allocator
                    .alloc_specific_and_lock(preempt_guard, parent.vm_space(), &range);

            parent.remove_mappings(&mut cursor, range.len(), &mut rs_as_delta)?;

            (offset, alloc_guard, cursor)
        }
        Some((offset, OffsetType::FixedNoReplace)) => {
            let range = offset..offset + map_size_bytes;

            let (alloc_guard, mut cursor) =
                parent
                    .allocator
                    .alloc_specific_and_lock(preempt_guard, parent.vm_space(), &range);

            if find_next_mapped!(cursor, range.end).is_some() {
                return_errno_with_message!(Errno::EEXIST, "the specified range is already mapped");
            }

            (offset, alloc_guard, cursor)
        }
        Some((offset, OffsetType::Hint)) => {
            let range = offset..offset + map_size_bytes;

            let (alloc_guard, mut cursor) =
                parent
                    .allocator
                    .alloc_specific_and_lock(preempt_guard, parent.vm_space(), &range);

            if find_next_mapped!(cursor, range.end).is_some() {
                drop(cursor);
                parent.allocator.alloc_and_lock(
                    preempt_guard,
                    parent.vm_space(),
                    map_size_bytes,
                    align,
                )?
            } else {
                (offset, alloc_guard, cursor)
            }
        }
    };

    #[cfg(debug_assertions)]
    {
        crate::vm::vmar::cursor_util::check_range_not_mapped(
            &mut cursor,
            map_to_addr..map_to_addr + map_size_bytes,
        );
    }

    Ok((map_to_addr, alloc_guard, cursor))
}

#[expect(clippy::too_many_arguments)]
fn build_vm_mapping(
    mappable: Option<Mappable>,
    path: Option<Path>,
    vmo: Option<Arc<Vmo>>,
    vmo_offset: usize,
    map_size: NonZeroUsize,
    map_to_addr: Vaddr,
    perms: VmPerms,
    mut may_perms: VmPerms,
    is_shared: bool,
    handle_page_faults_around: bool,
) -> Result<VmMapping> {
    // Parse the `Mappable` and prepare the `MappedMemory`.
    let (mapped_mem, inode) = if let Some(mappable) = mappable {
        // Handle the memory backed by device or page cache.
        match mappable {
            Mappable::Inode(inode) => {
                let is_writable_tracked = if let Some(memfd_inode) =
                    inode.downcast_ref::<MemfdInode>()
                    && is_shared
                    && may_perms.contains(VmPerms::MAY_WRITE)
                {
                    memfd_inode.check_writable(perms, &mut may_perms)?;
                    true
                } else {
                    false
                };

                // Since `Mappable::Inode` is provided, it is
                // reasonable to assume that the VMO is provided.
                let mapped_mem = MappedMemory::Vmo(MappedVmo::new(
                    vmo.unwrap(),
                    vmo_offset,
                    is_writable_tracked,
                )?);
                (mapped_mem, Some(inode))
            }
            Mappable::IoMem(iomem) => (MappedMemory::Device(iomem), None),
        }
    } else if let Some(vmo) = vmo {
        (
            MappedMemory::Vmo(MappedVmo::new(vmo, vmo_offset, false)?),
            None,
        )
    } else {
        (MappedMemory::Anonymous, None)
    };

    Ok(VmMapping::new(
        map_size,
        map_to_addr,
        mapped_mem,
        inode,
        path,
        is_shared,
        handle_page_faults_around,
        perms | may_perms,
    ))
}

fn readahead_for_populate(
    vmo: Option<Arc<Vmo>>,
    vmo_offset: usize,
    map_size: NonZeroUsize,
) -> Result<()> {
    if let Some(vmo) = vmo {
        for offset in (vmo_offset..vmo_offset + map_size.get()).step_by(PAGE_SIZE) {
            vmo.commit_on(offset / PAGE_SIZE, CommitFlags::empty())?;
        }
    }
    Ok(())
}

pub(super) fn map_to_page_table(cursor: &mut VmarCursorMut<'_>, vm_mapping: VmMapping) {
    let max_level = cursor.guard_level();
    for (mapping, level) in vm_mapping.split_for_pt(max_level) {
        cursor.jump(mapping.map_to_addr()).unwrap();
        cursor.adjust_level(level);

        map_to_page_table_recursive(cursor, mapping);
    }
}

// Inserts the mapping to the current page table frame's subtree recursively.
fn map_to_page_table_recursive(cursor: &mut VmarCursorMut<'_>, vm_mapping: VmMapping) {
    let mut vm_mapping = Some(vm_mapping);
    let cur_level = cursor.level();
    while let Some(remain) = vm_mapping.as_ref()
        && let Some(PteRangeMeta::ChildPt(r)) = cursor.aux_meta().inner.find(&remain.range()).next()
    {
        debug_assert!(is_intersected(&remain.range(), r));
        let child_start = r.start;

        let (left, child_mapping, right) = vm_mapping.take().unwrap().split_range(r);

        vm_mapping = right;

        if let Some(left) = left {
            cursor.aux_meta_mut().insert_try_merge(left);
        }

        cursor.jump(child_start).unwrap();
        cursor.push_level_if_exists().unwrap();

        map_to_page_table_recursive(cursor, child_mapping);

        cursor.adjust_level(cur_level);
    }

    if let Some(vm_mapping) = vm_mapping {
        cursor.aux_meta_mut().insert_try_merge(vm_mapping);
    }
}

pub(super) fn map_populate(cursor: &mut VmarCursorMut<'_>, vm_mapping: VmMapping) {
    // TODO: Support populating huge pages.
    for (mut mapping, level) in vm_mapping.split_for_pt(1) {
        let va = mapping.map_to_addr();
        cursor.jump(va).unwrap();
        debug_assert_eq!(level, 1);
        cursor.adjust_level(level);

        let map_end = va + mapping.map_size();
        let page_range = va..map_end;

        let flags = PageFlags::from(mapping.perms()) | PageFlags::ACCESSED;
        let map_prop = PageProperty::new_user(flags, CachePolicy::Writeback);

        match &mapping.mapped_mem() {
            MappedMemory::Vmo(vmo) => {
                for page in page_range.step_by(PAGE_SIZE) {
                    let offset = (page - va) / PAGE_SIZE;
                    let Ok(frame) = vmo.get_committed_frame(offset) else {
                        // Ignore errors here. If I/O is needed here, the page
                        // may get written back after `readahead_for_populate`
                        // due to reasons like memory pressure. Avoid trying
                        // again to avoid thrashing.
                        continue;
                    };
                    cursor.jump(page).unwrap();

                    // Make the mapping copy-on-write for private mappings.
                    let flags = if mapping.is_shared() {
                        flags - PageFlags::W
                    } else {
                        flags
                    };
                    let map_prop = PageProperty::new_user(flags, CachePolicy::Writeback);

                    cursor.map(frame, map_prop);
                }
            }
            MappedMemory::Anonymous => {
                for page in page_range.step_by(PAGE_SIZE) {
                    let Ok(frame) = FrameAllocOptions::new().alloc_frame() else {
                        // Ignore errors here for the same reason as above.
                        continue;
                    };
                    cursor.jump(page).unwrap();
                    cursor.map(frame.into(), map_prop);
                }
            }
            MappedMemory::Device(io_mem) => {
                cursor.map_iomem(io_mem.clone(), map_prop, page_range.len(), 0);
            }
        };

        mapping.set_fully_mapped();
        cursor.aux_meta_mut().insert_try_merge(mapping);
    }
}
