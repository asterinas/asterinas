// SPDX-License-Identifier: MPL-2.0

use core::num::NonZeroUsize;

use super::{MappedMemory, MappedVmo, RssDelta, VmMapping, Vmar};
use crate::{
    fs::{
        file::{FileLike, Mappable},
        ramfs::memfd::MemfdInode,
        vfs::path::Path,
    },
    prelude::*,
    vm::{perms::VmPerms, vmo::Vmo},
};

impl Vmar {
    /// Creates a mapping into the VMAR through a set of VMAR mapping options.
    ///
    /// # Examples
    ///
    /// ```
    /// use ostd::mm::PAGE_SIZE;
    ///
    /// use crate::{
    ///     current_userspace,
    ///     vm::{perms::VmPerms, vmar::VmarMapOffset, vmo::VmoOptions},
    /// };
    ///
    /// let vmo = VmoOptions::new(10 * PAGE_SIZE).alloc().unwrap();
    ///
    /// let target_vaddr = 0x1234000;
    /// let real_vaddr = current_userspace!()
    ///     .vmar()
    ///     // Create a read-only mapping spanning four pages
    ///     .new_map(PAGE_SIZE * 4, VmPerms::READ)
    ///     .unwrap()
    ///     // Provide an optional offset for the mapping inside the VMAR
    ///     .offset(VmarMapOffset::FixedNoReplace(target_vaddr))
    ///     // Specify an optional binding VMO
    ///     .vmo(vmo)
    ///     // Provide an optional offset to indicate the corresponding offset
    ///     // in the VMO for the mapping
    ///     .vmo_offset(2 * PAGE_SIZE)
    ///     .build()
    ///     .unwrap();
    ///
    /// assert_eq!(real_vaddr, target_vaddr);
    /// ```
    ///
    /// For more details on the available options, see [`VmarMapOptions`].
    pub fn new_map(&self, size: usize, perms: VmPerms) -> Result<VmarMapOptions<'_>> {
        Ok(VmarMapOptions::new(self, size, perms))
    }
}

/// Options for creating a new mapping.
pub struct VmarMapOptions<'a> {
    parent: &'a Vmar,
    mappable: Option<Mappable>,
    path: Option<Path>,
    perms: VmPerms,
    may_perms: VmPerms,
    vmo_offset: usize,
    size: usize,
    offset: VmarMapOffset,
    align: usize,
    // Whether the mapping is mapped with `MAP_SHARED`.
    is_shared: bool,
    // Whether the mapping needs to handle surrounding pages when handling
    // page fault.
    handle_page_faults_around: bool,
}

/// An offset within a VMAR where a new mapping will reside.
///
/// Note that this differs from the VMO offset. The VMO offset and the VMO
/// itself together specify the content of the mapping.
#[derive(Clone, Copy, Debug)]
pub enum VmarMapOffset {
    /// The new mapping will be placed at the specified offset. Conflict
    /// mappings will be replaced.
    FixedReplace(usize),
    /// The new mapping will be placed at the specified offset. Conflict
    /// mappings will cause an error.
    FixedNoReplace(usize),
    /// The new mapping may be placed at the specified offset if there are no
    /// conflict mappings.
    ///
    /// Otherwise, it can be placed at any available offset where there are no
    /// conflict mappings.
    Hint(usize),
    /// The new mapping can be placed at any available offset where there are
    /// no conflict mappings.
    Any,
}

impl<'a> VmarMapOptions<'a> {
    /// Creates a default set of options with the size and the memory access
    /// permissions.
    fn new(parent: &'a Vmar, size: usize, perms: VmPerms) -> Self {
        Self {
            parent,
            mappable: None,
            path: None,
            perms,
            may_perms: VmPerms::ALL_MAY_PERMS,
            vmo_offset: 0,
            size,
            offset: VmarMapOffset::Any,
            align: PAGE_SIZE,
            is_shared: false,
            handle_page_faults_around: false,
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
    /// set with a [`Mappable::Vmo`].
    ///
    /// # Panics
    ///
    /// This function panics if a [`Vmo`] or [`Mappable`] is already provided.
    pub fn vmo(mut self, vmo: Arc<Vmo>) -> Self {
        if self.mappable.is_some() {
            panic!("Cannot set `vmo` when `mappable` is already set");
        }
        self.mappable = Some(Mappable::Vmo(vmo));

        self
    }

    /// Sets the [`Path`] of the mapping.
    ///
    /// If a [`Vmo`] is specified, the inode behind the [`Path`] must have
    /// the [`Vmo`] as the page cache.
    ///
    /// The [`Path`] of a mapping will be implicitly set if [`Self::mappable`]
    /// is set.
    ///
    /// # Panics
    ///
    /// This function panics if a [`Path`] is already provided.
    pub fn path(mut self, path: Path) -> Self {
        if self.path.is_some() {
            panic!("Cannot set `path` when `path` is already set");
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
    pub fn offset(mut self, offset: VmarMapOffset) -> Self {
        self.offset = offset;
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

    /// Binds the file's [`Mappable`] object to the mapping and sets the
    /// [`Path`] of the mapping.
    ///
    /// This method accepts file-specific details, like a page cache (inode)
    /// or I/O memory, but not both simultaneously.
    ///
    /// # Panics
    ///
    /// This function panics if a [`Vmo`], [`Mappable`], or [`Path`] is already
    /// provided.
    ///
    /// # Errors
    ///
    /// This function returns an error if the file does not have a corresponding
    /// mappable object of [`Mappable`].
    pub fn mappable(mut self, file: &dyn FileLike) -> Result<Self> {
        if self.mappable.is_some() {
            panic!("Cannot set `mappable` when `mappable` is already set");
        }
        if self.path.is_some() {
            panic!("Cannot set `mappable` when `path` is already set");
        }

        let mappable = file.mappable()?;
        self.mappable = Some(mappable);
        self.path = Some(file.path().clone());

        Ok(self)
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
            mappable,
            path,
            perms,
            mut may_perms,
            vmo_offset,
            size: map_size,
            offset,
            align,
            is_shared,
            handle_page_faults_around,
        } = self;

        let mut inner = parent.inner.write();

        inner
            .check_extra_size_fits_rlimit(map_size)
            .or_else(|err| {
                if let VmarMapOffset::FixedReplace(map_to_addr) = offset {
                    // Overlapping mappings will be removed. Check again after considering this fact.
                    let expand_size =
                        map_size - inner.count_overlap_size(map_to_addr..map_to_addr + map_size);
                    inner.check_extra_size_fits_rlimit(expand_size)
                } else {
                    Err(err)
                }
            })?;

        // Allocates a free region.
        debug!(
            "allocate free region, map_size = 0x{:x}, offset = {:x?}, align = 0x{:x}",
            map_size, offset, align
        );
        let map_to_addr = match offset {
            VmarMapOffset::FixedReplace(map_to_addr) => {
                let mut rss_delta = RssDelta::new(parent);
                inner.alloc_free_region_exact_truncate(
                    parent.vm_space(),
                    map_to_addr,
                    map_size,
                    &mut rss_delta,
                )?;
                map_to_addr
            }
            VmarMapOffset::FixedNoReplace(map_to_addr) => {
                inner.alloc_free_region_exact(map_to_addr, map_size)?;
                map_to_addr
            }
            VmarMapOffset::Hint(map_to_addr) => {
                if inner.alloc_free_region_exact(map_to_addr, map_size).is_ok() {
                    map_to_addr
                } else {
                    inner.alloc_free_region(map_size, align)?.start
                }
            }
            VmarMapOffset::Any => inner.alloc_free_region(map_size, align)?.start,
        };

        // Parse the `Mappable` and prepare the `MappedMemory`.
        let (mapped_mem, io_mem) = match mappable {
            Some(Mappable::Vmo(vmo)) => {
                if let Some(ref path) = path {
                    debug_assert!(Arc::ptr_eq(&vmo, &path.inode().page_cache().unwrap()));
                }

                let is_writable_tracked = if let Some(ref path) = path
                    && let Some(memfd_inode) = path.inode().downcast_ref::<MemfdInode>()
                    && is_shared
                    && may_perms.contains(VmPerms::MAY_WRITE)
                {
                    memfd_inode.check_writable(perms, &mut may_perms)?;
                    true
                } else {
                    false
                };

                let mapped_mem =
                    MappedMemory::Vmo(MappedVmo::new(vmo, vmo_offset, is_writable_tracked)?);
                (mapped_mem, None)
            }
            Some(Mappable::IoMem(io_mem)) => (MappedMemory::Device, Some(io_mem)),
            None => (MappedMemory::Anonymous, None),
        };

        // Build the mapping.
        let vm_mapping = VmMapping::new(
            NonZeroUsize::new(map_size).unwrap(),
            map_to_addr,
            mapped_mem,
            path,
            is_shared,
            handle_page_faults_around,
            perms | may_perms,
        );

        // Populate device memory if needed before adding to VMAR.
        //
        // We have to map before inserting the `VmMapping` into the tree,
        // otherwise another traversal is needed for locating the `VmMapping`.
        // Exchange the operation is ok since we hold the write lock on the
        // VMAR.
        if let Some(io_mem) = io_mem {
            vm_mapping.populate_device(parent.vm_space(), io_mem, vmo_offset);
        }

        // Add the mapping to the VMAR.
        inner.insert_try_merge(vm_mapping);

        Ok(map_to_addr)
    }

    /// Checks whether all options are valid.
    fn check_options(&self) -> Result<()> {
        // Check align.
        debug_assert!(self.align.is_multiple_of(PAGE_SIZE));
        debug_assert!(self.align.is_power_of_two());
        if !self.align.is_multiple_of(PAGE_SIZE) || !self.align.is_power_of_two() {
            return_errno_with_message!(Errno::EINVAL, "invalid align");
        }
        debug_assert!(self.size.is_multiple_of(self.align));
        if !self.size.is_multiple_of(self.align) {
            return_errno_with_message!(Errno::EINVAL, "invalid mapping size");
        }
        debug_assert!(self.vmo_offset.is_multiple_of(self.align));
        if !self.vmo_offset.is_multiple_of(self.align) {
            return_errno_with_message!(Errno::EINVAL, "invalid vmo offset");
        }
        match self.offset {
            VmarMapOffset::FixedReplace(offset)
            | VmarMapOffset::FixedNoReplace(offset)
            | VmarMapOffset::Hint(offset) => {
                debug_assert!(offset.is_multiple_of(self.align));
                if !offset.is_multiple_of(self.align) {
                    return_errno_with_message!(Errno::EINVAL, "invalid offset");
                }
            }
            VmarMapOffset::Any => (),
        }
        self.check_perms()
    }

    /// Checks whether the permissions of the mapping is valid.
    fn check_perms(&self) -> Result<()> {
        if !VmPerms::ALL_MAY_PERMS.contains(self.may_perms)
            || !VmPerms::ALL_PERMS.contains(self.perms)
        {
            return_errno_with_message!(Errno::EACCES, "invalid perms");
        }

        let vm_perms = self.perms | self.may_perms;
        vm_perms.check()
    }
}
