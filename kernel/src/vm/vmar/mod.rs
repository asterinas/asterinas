// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod interval_set;
mod static_cap;
pub mod vm_mapping;

use core::{num::NonZeroUsize, ops::Range};

use align_ext::AlignExt;
use aster_rights::Rights;
use ostd::mm::{tlb::TlbFlushOp, PageFlags, PageProperty, VmSpace, MAX_USERSPACE_VADDR};

use self::{
    interval_set::{Interval, IntervalSet},
    vm_mapping::{MappedVmo, VmMapping},
};
use crate::{
    prelude::*,
    process::{Process, ResourceType},
    thread::exception::PageFaultInfo,
    vm::{
        perms::VmPerms,
        vmo::{Vmo, VmoRightsOp},
    },
};

/// Virtual Memory Address Regions (VMARs) are a type of capability that manages
/// user address spaces.
///
/// # Capabilities
///
/// As a capability, each VMAR is associated with a set of access rights,
/// whose semantics are explained below.
///
/// The semantics of each access rights for VMARs are described below:
///  * The Dup right allows duplicating a VMAR.
///  * The Read, Write, Exec rights allow creating memory mappings with
///    readable, writable, and executable access permissions, respectively.
///  * The Read and Write rights allow the VMAR to be read from and written to
///    directly.
///
/// VMARs are implemented with two flavors of capabilities:
/// the dynamic one (`Vmar<Rights>`) and the static one (`Vmar<R: TRights>`).
pub struct Vmar<R = Rights>(Arc<Vmar_>, R);

pub trait VmarRightsOp {
    /// Returns the access rights.
    fn rights(&self) -> Rights;

    /// Checks whether current rights meet the input `rights`.
    fn check_rights(&self, rights: Rights) -> Result<()> {
        if self.rights().contains(rights) {
            Ok(())
        } else {
            return_errno_with_message!(Errno::EACCES, "VMAR rights are insufficient");
        }
    }
}

impl<R> PartialEq for Vmar<R> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<R> Vmar<R> {
    /// FIXME: This function should require access control
    pub fn vm_space(&self) -> &Arc<VmSpace> {
        self.0.vm_space()
    }

    /// Resizes the original mapping.
    ///
    /// The range of the mapping goes from `map_addr..map_addr + old_size` to
    /// `map_addr..map_addr + new_size`.
    ///
    /// The range of the original mapping does not have to solely map to a
    /// whole [`VmMapping`], but it must ensure that all existing ranges have a
    /// mapping. Otherwise, this method will return `Err`.
    ///
    /// If the new mapping size is smaller than the original mapping size, the
    /// extra part will be unmapped. If the new mapping is larger than the old
    /// mapping and the extra part overlaps with existing mapping, resizing
    /// will fail and return `Err`.
    pub fn resize_mapping(&self, map_addr: Vaddr, old_size: usize, new_size: usize) -> Result<()> {
        self.0.resize_mapping(map_addr, old_size, new_size)
    }
}

pub(super) struct Vmar_ {
    /// VMAR inner
    inner: RwMutex<VmarInner>,
    /// The offset relative to the root VMAR
    base: Vaddr,
    /// The total size of the VMAR in bytes
    size: usize,
    /// The attached `VmSpace`
    vm_space: Arc<VmSpace>,
}

struct VmarInner {
    /// The mapped pages and associated metadata.
    vm_mappings: IntervalSet<Vaddr, VmMapping>,
    /// The total mapped memory in bytes.
    total_vm: usize,
}

impl VmarInner {
    const fn new() -> Self {
        Self {
            vm_mappings: IntervalSet::new(),
            total_vm: 0,
        }
    }

    /// Returns `Ok` if the calling process may expand its mapped
    /// memory by the passed size.
    fn check_expand_size(&mut self, expand_size: usize) -> Result<()> {
        let Some(process) = Process::current() else {
            // When building a `Process`, the kernel task needs to build
            // some `VmMapping`s, in which case this branch is reachable.
            return Ok(());
        };

        let rlimt_as = process
            .resource_limits()
            .get_rlimit(ResourceType::RLIMIT_AS)
            .get_cur();

        let new_total_vm = self
            .total_vm
            .checked_add(expand_size)
            .ok_or(Errno::ENOMEM)?;
        if new_total_vm > rlimt_as as usize {
            return_errno_with_message!(Errno::ENOMEM, "address space limit overflow");
        }
        Ok(())
    }

    /// Inserts a `VmMapping` into the `Vmar`.
    ///
    /// Make sure the insertion doesn't exceed address space limit.
    fn insert(&mut self, vm_mapping: VmMapping) {
        self.total_vm += vm_mapping.map_size();
        self.vm_mappings.insert(vm_mapping);
    }

    /// Removes a `VmMapping` based on the provided key from the `Vmar`.
    fn remove(&mut self, key: &Vaddr) -> Option<VmMapping> {
        let vm_mapping = self.vm_mappings.remove(key)?;
        self.total_vm -= vm_mapping.map_size();
        Some(vm_mapping)
    }

    /// Calculates the total amount of overlap between `VmMapping`s
    /// and the provided range.
    fn count_overlap_size(&self, range: Range<Vaddr>) -> usize {
        let mut sum_overlap_size = 0;
        for vm_mapping in self.vm_mappings.find(&range) {
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);
            sum_overlap_size += intersected_range.end - intersected_range.start;
        }
        sum_overlap_size
    }

    /// Allocates a free region for mapping with a specific offset and size.
    ///
    /// If the provided range is already occupied, return an error.
    fn alloc_free_region_exact(&mut self, offset: Vaddr, size: usize) -> Result<Range<Vaddr>> {
        if self
            .vm_mappings
            .find(&(offset..offset + size))
            .next()
            .is_some()
        {
            return_errno_with_message!(Errno::EACCES, "Requested region is already occupied");
        }

        Ok(offset..(offset + size))
    }

    /// Allocates a free region for mapping with a specific offset and size.
    ///
    /// If the provided range is already occupied, this function truncates all
    /// the mappings that intersect with the range.
    fn alloc_free_region_exact_truncate(
        &mut self,
        vm_space: &VmSpace,
        offset: Vaddr,
        size: usize,
    ) -> Result<Range<Vaddr>> {
        let range = offset..offset + size;
        let mut mappings_to_remove = Vec::new();
        for vm_mapping in self.vm_mappings.find(&range) {
            mappings_to_remove.push(vm_mapping.map_to_addr());
        }

        for vm_mapping_addr in mappings_to_remove {
            let vm_mapping = self.remove(&vm_mapping_addr).unwrap();
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            let (left, taken, right) = vm_mapping.split_range(&intersected_range)?;
            if let Some(left) = left {
                self.insert(left);
            }
            if let Some(right) = right {
                self.insert(right);
            }

            taken.unmap(vm_space)?;
        }

        Ok(offset..(offset + size))
    }

    /// Allocates a free region for mapping.
    ///
    /// If no such region is found, return an error.
    fn alloc_free_region(&mut self, size: usize, align: usize) -> Result<Range<Vaddr>> {
        // Fast path that there's still room to the end.
        let highest_occupied = self
            .vm_mappings
            .iter()
            .next_back()
            .map_or(ROOT_VMAR_LOWEST_ADDR, |vm_mapping| vm_mapping.range().end);
        // FIXME: The up-align may overflow.
        let last_occupied_aligned = highest_occupied.align_up(align);
        if let Some(last) = last_occupied_aligned.checked_add(size) {
            if last <= ROOT_VMAR_CAP_ADDR {
                return Ok(last_occupied_aligned..last);
            }
        }

        // Slow path that we need to search for a free region.
        // Here, we use a simple brute-force FIRST-FIT algorithm.
        // Allocate as low as possible to reduce fragmentation.
        let mut last_end: Vaddr = ROOT_VMAR_LOWEST_ADDR;
        for vm_mapping in self.vm_mappings.iter() {
            let range = vm_mapping.range();

            debug_assert!(range.start >= last_end);
            debug_assert!(range.end <= highest_occupied);

            let last_aligned = last_end.align_up(align);
            let needed_end = last_aligned
                .checked_add(size)
                .ok_or(Error::new(Errno::ENOMEM))?;

            if needed_end <= range.start {
                return Ok(last_aligned..needed_end);
            }

            last_end = range.end;
        }

        return_errno_with_message!(Errno::ENOMEM, "Cannot find free region for mapping");
    }
}

pub const ROOT_VMAR_LOWEST_ADDR: Vaddr = 0x001_0000; // 64 KiB is the Linux configurable default
const ROOT_VMAR_CAP_ADDR: Vaddr = MAX_USERSPACE_VADDR;

/// Returns whether the input `vaddr` is a legal user space virtual address.
pub fn is_userspace_vaddr(vaddr: Vaddr) -> bool {
    (ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_CAP_ADDR).contains(&vaddr)
}

impl Interval<usize> for Arc<Vmar_> {
    fn range(&self) -> Range<usize> {
        self.base..(self.base + self.size)
    }
}

impl Vmar_ {
    fn new(inner: VmarInner, vm_space: Arc<VmSpace>, base: usize, size: usize) -> Arc<Self> {
        Arc::new(Vmar_ {
            inner: RwMutex::new(inner),
            base,
            size,
            vm_space,
        })
    }

    fn new_root() -> Arc<Self> {
        let vmar_inner = VmarInner::new();
        let vm_space = VmSpace::new();
        Vmar_::new(vmar_inner, Arc::new(vm_space), 0, ROOT_VMAR_CAP_ADDR)
    }

    fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        assert!(range.start % PAGE_SIZE == 0);
        assert!(range.end % PAGE_SIZE == 0);
        self.do_protect_inner(perms, range)?;
        Ok(())
    }

    // Do real protect. The protected range is ensured to be mapped.
    fn do_protect_inner(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        let mut inner = self.inner.write();
        let vm_space = self.vm_space();

        let mut protect_mappings = Vec::new();

        for vm_mapping in inner.vm_mappings.find(&range) {
            protect_mappings.push((vm_mapping.map_to_addr(), vm_mapping.perms()));
        }

        for (vm_mapping_addr, vm_mapping_perms) in protect_mappings {
            if perms == vm_mapping_perms {
                continue;
            }
            let vm_mapping = inner.remove(&vm_mapping_addr).unwrap();
            let vm_mapping_range = vm_mapping.range();
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);

            // Protects part of the taken `VmMapping`.
            let (left, taken, right) = vm_mapping.split_range(&intersected_range)?;

            let taken = taken.protect(vm_space.as_ref(), perms);
            inner.insert(taken);

            // And put the rest back.
            if let Some(left) = left {
                inner.insert(left);
            }
            if let Some(right) = right {
                inner.insert(right);
            }
        }

        Ok(())
    }

    /// Handles user space page fault, if the page fault is successfully handled, return Ok(()).
    pub fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        let address = page_fault_info.address;
        if !(self.base..self.base + self.size).contains(&address) {
            return_errno_with_message!(Errno::EACCES, "page fault addr is not in current vmar");
        }

        let inner = self.inner.read();

        if let Some(vm_mapping) = inner.vm_mappings.find_one(&address) {
            debug_assert!(vm_mapping.range().contains(&address));
            return vm_mapping.handle_page_fault(&self.vm_space, page_fault_info);
        }

        return_errno_with_message!(Errno::EACCES, "page fault addr is not in current vmar");
    }

    /// Clears all content of the root VMAR.
    fn clear_root_vmar(&self) -> Result<()> {
        self.vm_space.clear().unwrap();
        let mut inner = self.inner.write();
        inner.vm_mappings.clear();
        Ok(())
    }

    pub fn remove_mapping(&self, range: Range<usize>) -> Result<()> {
        let mut inner = self.inner.write();
        inner.alloc_free_region_exact_truncate(&self.vm_space, range.start, range.len())?;
        Ok(())
    }

    // Split and unmap the found mapping if resize smaller.
    // Enlarge the last mapping if resize larger.
    fn resize_mapping(&self, map_addr: Vaddr, old_size: usize, new_size: usize) -> Result<()> {
        debug_assert!(map_addr % PAGE_SIZE == 0);
        debug_assert!(old_size % PAGE_SIZE == 0);
        debug_assert!(new_size % PAGE_SIZE == 0);

        if new_size == 0 {
            return_errno_with_message!(Errno::EINVAL, "can not resize a mapping to 0 size");
        }

        if new_size == old_size {
            return Ok(());
        }

        let old_map_end = map_addr + old_size;
        let new_map_end = map_addr + new_size;

        if new_size < old_size {
            self.remove_mapping(new_map_end..old_map_end)?;
            return Ok(());
        }

        let mut inner = self.inner.write();
        let last_mapping = inner.vm_mappings.find_one(&(old_map_end - 1)).unwrap();
        let last_mapping_addr = last_mapping.map_to_addr();
        let extra_mapping_start = last_mapping.map_end();

        inner.check_expand_size(new_map_end - extra_mapping_start)?;

        let last_mapping = inner.remove(&last_mapping_addr).unwrap();
        inner.alloc_free_region_exact(extra_mapping_start, new_map_end - extra_mapping_start)?;
        let last_mapping = last_mapping.enlarge(new_map_end - extra_mapping_start);
        inner.insert(last_mapping);
        Ok(())
    }

    /// Returns the attached `VmSpace`.
    fn vm_space(&self) -> &Arc<VmSpace> {
        &self.vm_space
    }

    pub(super) fn new_fork_root(self: &Arc<Self>) -> Result<Arc<Self>> {
        let new_vmar_ = {
            let vmar_inner = VmarInner::new();
            let new_space = VmSpace::new();
            Vmar_::new(vmar_inner, Arc::new(new_space), self.base, self.size)
        };

        {
            let inner = self.inner.read();
            let mut new_inner = new_vmar_.inner.write();

            // Clone mappings.
            let new_vmspace = new_vmar_.vm_space();
            let range = self.base..(self.base + self.size);
            let mut new_cursor = new_vmspace.cursor_mut(&range).unwrap();
            let cur_vmspace = self.vm_space();
            let mut cur_cursor = cur_vmspace.cursor_mut(&range).unwrap();
            for vm_mapping in inner.vm_mappings.iter() {
                let base = vm_mapping.map_to_addr();

                // Clone the `VmMapping` to the new VMAR.
                let new_mapping = vm_mapping.new_fork()?;
                new_inner.insert(new_mapping);

                // Protect the mapping and copy to the new page table for COW.
                cur_cursor.jump(base).unwrap();
                new_cursor.jump(base).unwrap();
                let mut op = |page: &mut PageProperty| {
                    page.flags -= PageFlags::W;
                };
                new_cursor.copy_from(&mut cur_cursor, vm_mapping.map_size(), &mut op);
            }
            cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::All);
            cur_cursor.flusher().dispatch_tlb_flush();
            cur_cursor.flusher().sync_tlb_flush();
        }

        Ok(new_vmar_)
    }
}

impl<R> Vmar<R> {
    /// The base address, i.e., the offset relative to the root VMAR.
    ///
    /// The base address of a root VMAR is zero.
    pub fn base(&self) -> Vaddr {
        self.0.base
    }

    /// The size of the VMAR in bytes.
    pub fn size(&self) -> usize {
        self.0.size
    }
}

/// Options for creating a new mapping. The mapping is not allowed to overlap
/// with any child VMARs. And unless specified otherwise, it is not allowed
/// to overlap with any existing mapping, either.
pub struct VmarMapOptions<'a, R1, R2> {
    parent: &'a Vmar<R1>,
    vmo: Option<Vmo<R2>>,
    perms: VmPerms,
    vmo_offset: usize,
    vmo_limit: usize,
    size: usize,
    offset: Option<usize>,
    align: usize,
    can_overwrite: bool,
    // Whether the mapping is mapped with `MAP_SHARED`
    is_shared: bool,
    // Whether the mapping needs to handle surrounding pages when handling page fault.
    handle_page_faults_around: bool,
}

impl<'a, R1, R2> VmarMapOptions<'a, R1, R2> {
    /// Creates a default set of options with the VMO and the memory access
    /// permissions.
    ///
    /// The VMO must have access rights that correspond to the memory
    /// access permissions. For example, if `perms` contains `VmPerms::Write`,
    /// then `vmo.rights()` should contain `Rights::WRITE`.
    pub fn new(parent: &'a Vmar<R1>, size: usize, perms: VmPerms) -> Self {
        Self {
            parent,
            vmo: None,
            perms,
            vmo_offset: 0,
            vmo_limit: usize::MAX,
            size,
            offset: None,
            align: PAGE_SIZE,
            can_overwrite: false,
            is_shared: false,
            handle_page_faults_around: false,
        }
    }

    /// Binds a VMO to the mapping.
    ///
    /// If the mapping is a private mapping, its size may not be equal to that of the VMO.
    /// For example, it is ok to create a mapping whose size is larger than
    /// that of the VMO, although one cannot read from or write to the
    /// part of the mapping that is not backed by the VMO.
    ///
    /// So you may wonder: what is the point of supporting such _oversized_
    /// mappings?  The reason is two-fold.
    ///  1. VMOs are resizable. So even if a mapping is backed by a VMO whose
    ///     size is equal to that of the mapping initially, we cannot prevent
    ///     the VMO from shrinking.
    ///  2. Mappings are not allowed to overlap by default. As a result,
    ///     oversized mappings can serve as a placeholder to prevent future
    ///     mappings from occupying some particular address ranges accidentally.
    pub fn vmo(mut self, vmo: Vmo<R2>) -> Self {
        self.vmo = Some(vmo);

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

    /// Sets the access limit offset for the binding VMO.
    pub fn vmo_limit(mut self, limit: usize) -> Self {
        self.vmo_limit = limit;
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
    pub fn offset(mut self, offset: usize) -> Self {
        self.offset = Some(offset);
        self
    }

    /// Sets whether the mapping can overwrite existing mappings.
    ///
    /// The default value is false.
    ///
    /// If this option is set to true, then the `offset` option must be
    /// set.
    pub fn can_overwrite(mut self, can_overwrite: bool) -> Self {
        self.can_overwrite = can_overwrite;
        self
    }

    /// Sets whether the mapping can be shared with other process.
    ///
    /// The default value is false.
    ///
    /// If this value is set to true, the mapping will be shared with child
    /// process when forking.
    pub fn is_shared(mut self, is_shared: bool) -> Self {
        self.is_shared = is_shared;
        self
    }

    /// Sets the mapping to handle surrounding pages when handling page fault.
    pub fn handle_page_faults_around(mut self) -> Self {
        self.handle_page_faults_around = true;
        self
    }
}

impl<'a, R1, R2> VmarMapOptions<'a, R1, R2>
where
    Vmo<R2>: VmoRightsOp,
{
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
            perms,
            vmo_offset,
            vmo_limit,
            size: map_size,
            offset,
            align,
            can_overwrite,
            is_shared,
            handle_page_faults_around,
        } = self;

        let mut inner = parent.0.inner.write();

        inner.check_expand_size(map_size).or_else(|e| {
            if can_overwrite {
                let offset = offset.ok_or(Error::with_message(
                    Errno::EINVAL,
                    "offset cannot be None since can overwrite is set",
                ))?;
                // MAP_FIXED may remove pages overlapped with requested mapping.
                let expand_size = map_size - inner.count_overlap_size(offset..offset + map_size);
                inner.check_expand_size(expand_size)
            } else {
                Err(e)
            }
        })?;

        // Allocates a free region.
        trace!("allocate free region, map_size = 0x{:x}, offset = {:x?}, align = 0x{:x}, can_overwrite = {}", map_size, offset, align, can_overwrite);
        let map_to_addr = if can_overwrite {
            // If can overwrite, the offset is ensured not to be `None`.
            let offset = offset.ok_or(Error::with_message(
                Errno::EINVAL,
                "offset cannot be None since can overwrite is set",
            ))?;
            inner.alloc_free_region_exact_truncate(parent.vm_space(), offset, map_size)?;
            offset
        } else if let Some(offset) = offset {
            inner.alloc_free_region_exact(offset, map_size)?;
            offset
        } else {
            let free_region = inner.alloc_free_region(map_size, align)?;
            free_region.start
        };

        // Build the mapping.
        let vmo = vmo.map(|vmo| MappedVmo::new(vmo.to_dyn(), vmo_offset..vmo_limit));
        let vm_mapping = VmMapping::new(
            NonZeroUsize::new(map_size).unwrap(),
            map_to_addr,
            vmo,
            is_shared,
            handle_page_faults_around,
            perms,
        );

        // Add the mapping to the VMAR.
        inner.insert(vm_mapping);

        Ok(map_to_addr)
    }

    /// Checks whether all options are valid.
    fn check_options(&self) -> Result<()> {
        // Check align.
        debug_assert!(self.align % PAGE_SIZE == 0);
        debug_assert!(self.align.is_power_of_two());
        if self.align % PAGE_SIZE != 0 || !self.align.is_power_of_two() {
            return_errno_with_message!(Errno::EINVAL, "invalid align");
        }
        debug_assert!(self.size % self.align == 0);
        if self.size % self.align != 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid mapping size");
        }
        debug_assert!(self.vmo_offset % self.align == 0);
        if self.vmo_offset % self.align != 0 {
            return_errno_with_message!(Errno::EINVAL, "invalid vmo offset");
        }
        if let Some(offset) = self.offset {
            debug_assert!(offset % self.align == 0);
            if offset % self.align != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid offset");
            }
        }
        self.check_perms()?;
        Ok(())
    }

    /// Checks whether the permissions of the mapping is subset of vmo rights.
    fn check_perms(&self) -> Result<()> {
        let Some(vmo) = &self.vmo else {
            return Ok(());
        };

        let perm_rights = Rights::from(self.perms);
        vmo.check_rights(perm_rights)
    }
}

/// Determines whether two ranges are intersected.
/// returns false if one of the ranges has a length of 0
pub fn is_intersected(range1: &Range<usize>, range2: &Range<usize>) -> bool {
    range1.start.max(range2.start) < range1.end.min(range2.end)
}

/// Gets the intersection range of two ranges.
/// The two ranges should be ensured to be intersected.
pub fn get_intersected_range(range1: &Range<usize>, range2: &Range<usize>) -> Range<usize> {
    debug_assert!(is_intersected(range1, range2));
    range1.start.max(range2.start)..range1.end.min(range2.end)
}
