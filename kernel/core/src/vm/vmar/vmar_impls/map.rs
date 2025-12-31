// SPDX-License-Identifier: MPL-2.0

use core::{num::NonZeroUsize, ops::Range};

use ostd::{
    mm::{CachePolicy, FrameAllocOptions, PageFlags, PageProperty},
    task::{DisabledPreemptGuard, disable_preempt},
};

use super::{MappedMemory, MappedVmo, RsAsDelta, VmMapping, Vmar};
use crate::{
    fs::{
        file::{FileLike, MappableObject},
        ramfs::memfd::MemfdInode,
    },
    prelude::*,
    vm::{
        page_cache::{Vmo, VmoMapMode},
        perms::VmPerms,
        vmar::{
            MapHandle,
            cursor::CursorExt,
            interval_set::Interval,
            is_userspace_vaddr_range,
            util::is_intersected,
            vm_allocator::AllocatorGuard,
            vm_mapping::{DeviceMapping, map_device_pages},
            vmar_impls::{PteRangeMeta, VmarCursorMut},
        },
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
    /// use crate::vm::{page_cache::VmoOptions, perms::VmPerms, vmar::Vmar};
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
    pub(crate) fn new_map<'b>(
        &self,
        size: NonZeroUsize,
        perms: VmPerms,
    ) -> Result<VmarMapOptions<'_, 'b>> {
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
    pub(crate) fn reserve(&self, size: NonZeroUsize, align: usize) -> Result<Vaddr> {
        assert!(align.is_power_of_two() && align.is_multiple_of(PAGE_SIZE));
        self.new_map(size, VmPerms::empty())?.align(align).build()
    }

    /// Reserves a specific range to exclude it from future allocations.
    ///
    /// See [`Self::reserve`] for details.
    ///
    /// The function returns [`Errno::ENOMEM`] if the range is already reserved
    /// or allocated.
    pub(crate) fn reserve_specific(&self, range: Range<Vaddr>) -> Result<()> {
        self.new_map(
            NonZeroUsize::new(range.end - range.start).unwrap(),
            VmPerms::empty(),
        )?
        .offset(range.start, OffsetType::FixedNoReplace)
        .build()
        .map(|_| ())
    }
}

/// Options for creating a new mapping.
pub(crate) struct VmarMapOptions<'a, 'b> {
    parent: &'a Vmar,
    mappable: Option<MappableObject<'b>>,
    file: Option<Arc<dyn FileLike>>,
    perms: VmPerms,
    may_perms: VmPerms,
    vmo_offset: usize,
    size: NonZeroUsize,
    offset: Option<(usize, OffsetType)>,
    align: usize,
    /// The sharing mode of the mapping.
    map_mode: MmapMode,
    /// Whether the mapping needs to handle surrounding pages when handling
    /// page fault.
    handle_page_faults_around: bool,
    // Whether to map all pages immediately instead of on-demand.
    populate: bool,
}

impl<'a, 'b> VmarMapOptions<'a, 'b> {
    /// Creates a default set of options with the size and the memory access
    /// permissions.
    pub(crate) fn new(parent: &'a Vmar, size: NonZeroUsize, perms: VmPerms) -> Self {
        Self {
            parent,
            mappable: None,
            file: None,
            perms,
            may_perms: VmPerms::ALL_MAY_PERMS,
            vmo_offset: 0,
            size,
            offset: None,
            align: PAGE_SIZE,
            map_mode: MmapMode::Private,
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
    pub(crate) fn may_perms(mut self, may_perms: VmPerms) -> Self {
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
    /// set with a [`MappableObject::Vmo`].
    ///
    /// # Panics
    ///
    /// This function panics if a [`Vmo`] or [`Mappable`] is already provided.
    ///
    /// [`Mappable`]: crate::fs::file::Mappable
    pub(crate) fn vmo(mut self, vmo: Arc<Vmo>) -> Self {
        if self.mappable.is_some() {
            panic!("Cannot set `vmo` when `mappable` is already set");
        }
        self.mappable = Some(MappableObject::Vmo(vmo));

        self
    }

    /// Sets the offset of the first memory page in the VMO that is to be
    /// mapped into the VMAR.
    ///
    /// The offset must be page-aligned and within the VMO.
    ///
    /// The default value is zero.
    pub(crate) fn vmo_offset(mut self, offset: usize) -> Self {
        self.vmo_offset = offset;
        self
    }

    /// Sets the mapping's alignment.
    ///
    /// The default value is the page size.
    ///
    /// The provided alignment must be a power of two and a multiple of the
    /// page size.
    pub(crate) fn align(mut self, align: usize) -> Self {
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
    pub(crate) fn offset(mut self, offset: usize, typ: OffsetType) -> Self {
        self.offset = Some((offset, typ));
        self
    }

    /// Sets the sharing mode of the mapping.
    ///
    /// The default value is [`MmapMode::Private`].
    ///
    /// A shared mapping is shared with child processes when forking.
    pub(crate) fn map_mode(mut self, map_mode: MmapMode) -> Self {
        self.map_mode = map_mode;
        self
    }

    /// Sets the mapping to handle surrounding pages when handling page fault.
    pub(crate) fn handle_page_faults_around(mut self) -> Self {
        self.handle_page_faults_around = true;
        self
    }

    /// Binds the file's [`MappableObject`] to the mapping and sets the file of
    /// the mapping.
    ///
    /// This method accepts file-specific details, like a page cache (inode) or
    /// I/O memory, but not both simultaneously.
    ///
    /// # Panics
    ///
    /// This function panics if a [`Vmo`], [`Mappable`], or file is already
    /// provided.
    ///
    /// # Errors
    ///
    /// This function returns an error if the file does not have a corresponding
    /// mappable object of [`Mappable`].
    ///
    /// [`Mappable`]: crate::fs::file::Mappable
    pub(crate) fn mappable(mut self, file: &'b Arc<dyn FileLike>) -> Result<Self> {
        if self.mappable.is_some() {
            panic!("Cannot set `mappable` when `mappable` is already set");
        }
        if self.file.is_some() {
            panic!("Cannot set `mappable` when `file` is already set");
        }

        let mappable = file.mappable(FileMmapRequest::new(self.map_mode.is_shared()))?;
        self.mappable = Some(mappable);
        self.file = Some(file.clone());

        Ok(self)
    }

    /// Sets whether to populate all pages immediately instead of on-demand.
    pub(crate) fn populate(mut self) -> Self {
        self.populate = true;
        self
    }

    /// Creates the mapping and adds it to the parent VMAR.
    ///
    /// All options will be checked at this point.
    ///
    /// On success, the virtual address of the new mapping is returned.
    pub(crate) fn build(mut self) -> Result<Vaddr> {
        self.check_options()?;

        if (self.perms & VmPerms::ALL_PERMS).is_empty() {
            // Linux leaves `PROT_NONE` mappings unpopulated even when
            // `MAP_POPULATE` is specified.
            self.populate = false;
        }

        let (mapped_mem, device_pages) = self.prepare_mapped_memory()?;

        let Self {
            parent,
            file,
            mappable: _,
            perms,
            may_perms,
            vmo_offset: _,
            size: map_size,
            offset,
            align,
            map_mode,
            handle_page_faults_around,
            populate,
        } = self;

        let replaces_existing = matches!(offset, Some((_, OffsetType::Fixed)));

        let preempt_guard = disable_preempt();

        let (map_to_addr, alloc_guard, mut cursor) =
            allocate_range(&preempt_guard, parent, offset, align, map_size)?;

        let affected_range = map_to_addr..map_to_addr + map_size.get();
        let old_rmap_entries = if replaces_existing {
            Vmar::snapshot_rmap_entries(&mut cursor, core::slice::from_ref(&affected_range))
        } else {
            Vec::new()
        };

        if replaces_existing {
            cursor.jump(affected_range.start).unwrap();
            let mut rs_as_delta = RsAsDelta::new(parent);
            if let Err(err) =
                parent.remove_mappings(&mut cursor, affected_range.len(), &mut rs_as_delta)
            {
                parent.refresh_rmap_entries(
                    &mut cursor,
                    old_rmap_entries,
                    core::slice::from_ref(&affected_range),
                );
                return Err(err);
            }
        }

        #[cfg(debug_assertions)]
        crate::vm::vmar::cursor::check_range_not_mapped(&mut cursor, affected_range.clone());

        debug!(
            "map_size = {:#x}, offset = {:x?}, align = {:#x}; allocated to {:#x}",
            map_size, offset, align, map_to_addr
        );
        let mut vm_mapping = VmMapping::new(
            map_size,
            map_to_addr,
            mapped_mem,
            file,
            map_mode,
            handle_page_faults_around,
            perms | may_perms,
        );

        if let Err(err) = parent.add_mapping_size(&preempt_guard, map_size.get()) {
            parent.refresh_rmap_entries(
                &mut cursor,
                old_rmap_entries,
                core::slice::from_ref(&affected_range),
            );
            return Err(err);
        }

        if matches!(vm_mapping.mapped_mem(), MappedMemory::Device(_)) {
            let frames_mapped = map_device_pages(&mut cursor, &affected_range, perms, device_pages);
            vm_mapping.add_frames_mapped(frames_mapped);
            parent.add_rss_counter(MapHandle::DEVICE_RSS_TYPE, frames_mapped as isize);
            map_to_page_table(&mut cursor, vm_mapping);
        } else if populate {
            let rss_type = vm_mapping.rss_type();
            let frames_mapped = map_populate(&mut cursor, vm_mapping);
            parent.add_rss_counter(rss_type, frames_mapped as isize);
        } else {
            map_to_page_table(&mut cursor, vm_mapping);
        }

        parent.refresh_rmap_entries(
            &mut cursor,
            old_rmap_entries,
            core::slice::from_ref(&affected_range),
        );

        drop(cursor);
        drop(alloc_guard);
        drop(preempt_guard);

        Ok(map_to_addr)
    }

    /// Checks whether all options are valid.
    fn check_options(&self) -> Result<()> {
        debug_assert!(self.align.is_multiple_of(PAGE_SIZE));
        debug_assert!(self.align.is_power_of_two());
        debug_assert!(self.size.get().is_multiple_of(self.align));
        debug_assert!(self.vmo_offset.is_multiple_of(self.align));

        if let Some((offset, _typ)) = self.offset {
            debug_assert!(offset.is_multiple_of(self.align));

            #[cfg(target_arch = "x86_64")]
            let is_empty_map32_hint = _typ == OffsetType::Map32Bit && offset == 0;
            #[cfg(not(target_arch = "x86_64"))]
            let is_empty_map32_hint = false;

            if !is_empty_map32_hint && !is_userspace_vaddr_range(offset, self.size.get()) {
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

    // Parse the `Mappable` and prepare the `MappedMemory`.
    //
    // This cannot be executed in the atomic mode since it may readahead pages.
    fn prepare_mapped_memory(&mut self) -> Result<(MappedMemory, Vec<DeviceMapping>)> {
        let mut device_pages = Vec::new();
        let mut new_may_perms = self.may_perms;

        let mem = match self.mappable.take() {
            Some(MappableObject::Vmo(vmo)) => {
                if let Some(file) = &self.file {
                    let path = file.path();
                    debug_assert!(Arc::ptr_eq(&vmo, &path.inode().page_cache().unwrap()));
                }

                let path = self.file.as_ref().map(|file| file.path());
                let is_writable_tracked = if let Some(path) = path
                    && let Some(memfd_inode) = path.inode().downcast_ref::<MemfdInode>()
                    && self.map_mode.is_shared()
                    && self.may_perms.contains(VmPerms::MAY_WRITE)
                {
                    memfd_inode.check_writable(self.perms, &mut new_may_perms)?;
                    true
                } else {
                    false
                };

                if self.populate {
                    readahead_for_populate(vmo.clone(), self.vmo_offset, self.size);
                }

                MappedMemory::Vmo(MappedVmo::new(vmo, self.vmo_offset, is_writable_tracked)?)
            }
            Some(MappableObject::Device(mappable)) => {
                if !self.map_mode.is_shared() {
                    return_errno_with_message!(
                        Errno::EINVAL,
                        "private device mappings are not yet supported"
                    );
                }
                let handle = MapHandle::new(self.size.get(), &mut device_pages);
                MappedMemory::Device(Arc::from(mappable.map(self.vmo_offset, handle)?))
            }
            None => MappedMemory::Anonymous,
        };

        self.may_perms = new_may_perms;

        Ok((mem, device_pages))
    }
}

/// The type of offset specified in [`VmarMapOptions::offset`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum OffsetType {
    Hint,
    Fixed,
    FixedNoReplace,
    #[cfg(target_arch = "x86_64")]
    Map32Bit,
}

fn allocate_range<'a>(
    preempt_guard: &'a DisabledPreemptGuard,
    parent: &'a Vmar,
    offset: Option<(usize, OffsetType)>,
    align: usize,
    map_size: NonZeroUsize,
) -> Result<(Vaddr, AllocatorGuard<'a>, VmarCursorMut<'a>)> {
    let map_size_bytes = map_size.get();

    let (map_to_addr, alloc_guard, cursor) = match offset {
        None => parent.allocator.alloc_and_lock(
            preempt_guard,
            parent.vm_space(),
            map_size_bytes,
            align,
        )?,
        Some((offset, OffsetType::Fixed)) => {
            let range = offset..offset + map_size_bytes;

            let (alloc_guard, cursor) =
                parent
                    .allocator
                    .alloc_specific_and_lock(preempt_guard, parent.vm_space(), &range);

            (offset, alloc_guard, cursor)
        }
        Some((offset, OffsetType::FixedNoReplace)) => {
            let range = offset..offset + map_size_bytes;

            let (alloc_guard, mut cursor) =
                parent
                    .allocator
                    .alloc_specific_and_lock(preempt_guard, parent.vm_space(), &range);

            if cursor.find_next_mapped(range.end).is_some() {
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

            if cursor.find_next_mapped(range.end).is_some() {
                drop(cursor);
                drop(alloc_guard);
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
        #[cfg(target_arch = "x86_64")]
        Some((offset, OffsetType::Map32Bit)) => {
            let allocation_range = crate::vm::vmar::VMAR_LOWEST_ADDR..super::MAP_32BIT_HIGH_LIMIT;

            if offset != 0
                && offset >= allocation_range.start
                && map_size_bytes <= allocation_range.end - offset
            {
                let range = offset..offset + map_size_bytes;
                let (alloc_guard, mut cursor) = parent.allocator.alloc_specific_and_lock(
                    preempt_guard,
                    parent.vm_space(),
                    &range,
                );

                if cursor.find_next_mapped(range.end).is_none() {
                    (offset, alloc_guard, cursor)
                } else {
                    drop(cursor);
                    drop(alloc_guard);
                    parent.allocator.alloc_and_lock_in_range(
                        preempt_guard,
                        parent.vm_space(),
                        &allocation_range,
                        map_size_bytes,
                        align,
                    )?
                }
            } else {
                parent.allocator.alloc_and_lock_in_range(
                    preempt_guard,
                    parent.vm_space(),
                    &allocation_range,
                    map_size_bytes,
                    align,
                )?
            }
        }
    };

    Ok((map_to_addr, alloc_guard, cursor))
}

fn readahead_for_populate(vmo: Arc<Vmo>, vmo_offset: usize, map_size: NonZeroUsize) {
    let end = (vmo_offset + map_size.get()).min(vmo.size());
    for offset in (vmo_offset..end).step_by(PAGE_SIZE) {
        // `MAP_POPULATE` is advisory. A page that cannot be read ahead can
        // still be populated by a later page fault.
        let _ = vmo.commit_on(offset / PAGE_SIZE, VmoMapMode::SharedRead);
    }
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

pub(super) fn map_populate(cursor: &mut VmarCursorMut<'_>, vm_mapping: VmMapping) -> usize {
    // TODO: Support populating huge pages.
    let mut total_frames_mapped = 0;
    for (mut mapping, level) in vm_mapping.split_for_pt(1) {
        let va = mapping.map_to_addr();
        cursor.jump(va).unwrap();
        debug_assert_eq!(level, 1);
        cursor.adjust_level(level);

        let map_end = va + mapping.map_size();
        let page_range = va..map_end;

        let flags = PageFlags::from(mapping.perms()) | PageFlags::ACCESSED;
        let map_prop = PageProperty::new_user(flags, CachePolicy::Writeback);
        let mut frames_mapped = 0;

        match mapping.mapped_mem() {
            MappedMemory::Vmo(vmo) => {
                for page in page_range.step_by(PAGE_SIZE) {
                    let offset = page - va;
                    let Ok((cache_page, mode)) =
                        vmo.get_committed_frame(offset, VmoMapMode::SharedRead)
                    else {
                        // Ignore errors here. If I/O is needed here, the page
                        // may get written back after `readahead_for_populate`
                        // due to reasons like memory pressure. Avoid trying
                        // again to avoid thrashing.
                        continue;
                    };
                    cursor.jump(page).unwrap();

                    // Make the mapping copy-on-write for private mappings.
                    let flags = if mapping.is_shared() && mode == VmoMapMode::SharedWrite {
                        flags
                    } else {
                        flags - PageFlags::W
                    };
                    let map_prop = PageProperty::new_user(flags, CachePolicy::Writeback);

                    cursor.map(cache_page.into(), map_prop);
                    frames_mapped += 1;
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
                    frames_mapped += 1;
                }
            }
            MappedMemory::Device(_) => {
                unreachable!("device pages are supplied by the mapping callback")
            }
        }

        mapping.add_frames_mapped(frames_mapped);
        total_frames_mapped += frames_mapped;
        cursor.aux_meta_mut().insert_try_merge(mapping);
    }

    total_frames_mapped
}

/// Properties of a virtual memory mapping that may affect file-specific behavior.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct FileMmapRequest {
    is_shared: bool,
}

impl FileMmapRequest {
    pub(crate) const fn new(is_shared: bool) -> Self {
        Self { is_shared }
    }

    /// Returns whether the mapping uses shared semantics.
    pub const fn is_shared(self) -> bool {
        self.is_shared
    }
}

/// The sharing mode of a virtual memory mapping.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum MmapMode {
    /// Private mappings use copy-on-write semantics.
    Private,
    /// Shared mappings propagate updates to the underlying mapped object.
    Shared,
}

impl MmapMode {
    pub(crate) const fn is_shared(self) -> bool {
        matches!(self, Self::Shared)
    }
}

#[cfg(ktest)]
mod test {
    use ostd::{io::IoMem, prelude::ktest};

    use super::*;
    use crate::{
        fs::{
            file::{Mappable, MappedObject},
            pseudofs::SockFs,
        },
        process::ProcessVm,
        vm::vmar::{RemapOldMappingAction, VmarHandle},
    };

    #[derive(Debug)]
    struct MixedDevice(Arc<()>);

    impl Mappable for MixedDevice {
        fn map(&self, _offset: usize, mut handle: MapHandle) -> Result<Box<dyn MappedObject>> {
            // No test accesses these I/O addresses; they are outside physical RAM.
            const IOMEM_START: usize = 0x200_000_000_000;
            static IO_MEM: spin::Once<IoMem> = spin::Once::new();
            let io_mem = IO_MEM
                .call_once(|| IoMem::acquire(IOMEM_START..IOMEM_START + 4 * 1024 * 1024).unwrap());
            let io_size = handle.size() - PAGE_SIZE;
            handle.map_frame(0, FrameAllocOptions::new().alloc_frame()?.into());
            handle.map_iomem(PAGE_SIZE, io_mem.slice(PAGE_SIZE..PAGE_SIZE + io_size));
            Ok(Box::new(Self(self.0.clone())))
        }
    }

    impl MappedObject for MixedDevice {
        fn dup_at_offset(&self, _offset: usize) -> Box<dyn MappedObject> {
            Box::new(Self(self.0.clone()))
        }
    }

    fn new_vmar() -> VmarHandle {
        crate::time::clocks::init_for_ktest();
        crate::util::random::init();
        VmarHandle::new(ProcessVm::new(SockFs::new_path())).unwrap()
    }

    fn map_mixed_device(vmar: &Vmar, addr: Vaddr, size: usize) -> Arc<()> {
        let device = MixedDevice(Arc::new(()));
        let mut options = vmar
            .new_map(
                NonZeroUsize::new(size).unwrap(),
                VmPerms::READ | VmPerms::WRITE,
            )
            .unwrap()
            .offset(addr, OffsetType::FixedNoReplace)
            .map_mode(MmapMode::Shared);
        options.mappable = Some(MappableObject::Device(&device));
        assert_eq!(options.build().unwrap(), addr);
        assert_eq!(vmar.get_rss_counter(MapHandle::DEVICE_RSS_TYPE), 1);
        device.0
    }

    // PR #2615: I/O pages must be unmapped without being subtracted from RAM RSS.
    #[ktest]
    fn device_unmap_counts_only_ram() {
        const ADDR: usize = 0x200_000;
        for size in [3 * PAGE_SIZE, 4 * 1024 * 1024] {
            let vmar = new_vmar();
            map_mixed_device(&vmar, ADDR, size);
            vmar.remove_mapping(ADDR + PAGE_SIZE..ADDR + 2 * PAGE_SIZE)
                .unwrap();
            assert_eq!(vmar.get_rss_counter(MapHandle::DEVICE_RSS_TYPE), 1);
            vmar.remove_mapping(ADDR..ADDR + size).unwrap();
            assert_eq!(vmar.get_rss_counter(MapHandle::DEVICE_RSS_TYPE), 0);
        }
    }

    #[ktest]
    fn device_remap_preserves_ram_rss() {
        const OLD_ADDR: usize = 0x200_000;
        const NEW_ADDR: usize = 0x800_000;
        let size = 3 * PAGE_SIZE;
        let vmar = new_vmar();
        map_mixed_device(&vmar, OLD_ADDR, size);
        assert_eq!(
            vmar.remap(
                OLD_ADDR,
                size,
                Some(NEW_ADDR),
                size,
                RemapOldMappingAction::Unmap
            )
            .unwrap(),
            NEW_ADDR
        );
        assert_eq!(vmar.get_rss_counter(MapHandle::DEVICE_RSS_TYPE), 1);
        vmar.remove_mapping(NEW_ADDR..NEW_ADDR + size).unwrap();
        assert_eq!(vmar.get_rss_counter(MapHandle::DEVICE_RSS_TYPE), 0);
    }

    // Dropping auxiliary metadata must not wait for page-table RCU reclamation.
    #[ktest]
    fn device_mapping_dropped_on_unmap_and_exit() {
        const ADDR: usize = 0x200_000;
        for size in [3 * PAGE_SIZE, 4 * 1024 * 1024] {
            let vmar = new_vmar();
            let mapped_object = map_mixed_device(&vmar, ADDR, size);
            assert!(Arc::strong_count(&mapped_object) > 1);
            vmar.remove_mapping(ADDR..ADDR + size).unwrap();
            assert_eq!(Arc::strong_count(&mapped_object), 1);

            let mapped_object = map_mixed_device(&vmar, ADDR, size);
            assert!(Arc::strong_count(&mapped_object) > 1);
            drop(vmar);
            assert_eq!(Arc::strong_count(&mapped_object), 1);
        }
    }
}
