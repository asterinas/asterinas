// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod interval;
mod options;
mod static_cap;
pub mod vm_mapping;

use core::ops::Range;

use align_ext::AlignExt;
use aster_rights::Rights;
use ostd::{
    cpu::CpuExceptionInfo,
    mm::{tlb::TlbFlushOp, PageFlags, PageProperty, VmSpace, MAX_USERSPACE_VADDR},
};

use self::{
    interval::{Interval, IntervalSet},
    vm_mapping::VmMapping,
};
use super::page_fault_handler::PageFaultHandler;
use crate::{
    prelude::*,
    thread::exception::{handle_page_fault_from_vm_space, PageFaultInfo},
    vm::perms::VmPerms,
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
///  * The Dup right allows duplicating a VMAR and creating children out of
///    a VMAR.
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
    fn check_rights(&self, rights: Rights) -> Result<()>;
}

impl<R> PartialEq for Vmar<R> {
    fn eq(&self, other: &Self) -> bool {
        Arc::ptr_eq(&self.0, &other.0)
    }
}

impl<R> VmarRightsOp for Vmar<R> {
    default fn rights(&self) -> Rights {
        unimplemented!()
    }

    default fn check_rights(&self, rights: Rights) -> Result<()> {
        if self.rights().contains(rights) {
            Ok(())
        } else {
            return_errno_with_message!(Errno::EACCES, "Rights check failed");
        }
    }
}

// TODO: how page faults can be delivered to and handled by the current VMAR.
impl<R> PageFaultHandler for Vmar<R> {
    default fn handle_page_fault(&self, _page_fault_info: &PageFaultInfo) -> Result<()> {
        unimplemented!()
    }
}

impl<R> Vmar<R> {
    /// FIXME: This function should require access control
    pub fn vm_space(&self) -> &Arc<VmSpace> {
        self.0.vm_space()
    }

    /// Resizes the original mapping `map_addr..map_addr + old_size` to `map_addr..map_addr + new_size`.
    ///
    /// The range of the original mapping does not have to correspond to the entire `VmMapping`,
    /// but it must ensure that all existing ranges have a mapping. Otherwise, this method will return `Err`.
    /// If the new mapping size is smaller than the original mapping size, the extra part will be unmapped.
    /// If the new mapping is larger than the old mapping and the extra part overlaps with existing mapping,
    /// resizing will fail and return `Err`.
    ///
    /// TODO: implement `remap` function to handle the case of overlapping mappings.
    /// If the overlapping mappings are not fixed, they can be moved to make the resizing mapping successful.
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
    /// The parent VMAR. If points to none, this is a root VMAR
    parent: Weak<Vmar_>,
}

struct VmarInner {
    /// Whether the VMAR is destroyed
    is_destroyed: bool,
    /// The child VMARs. The key is offset relative to root VMAR
    child_vmar_s: BTreeMap<Vaddr, Arc<Vmar_>>,
    /// The mapped VMOs. The key is offset relative to root VMAR
    vm_mappings: BTreeMap<Vaddr, Arc<VmMapping>>,
    /// Free regions that can be used for creating child VMAR or mapping VMOs
    free_regions: BTreeMap<Vaddr, FreeRegion>,
}

impl VmarInner {
    const fn new() -> Self {
        Self {
            is_destroyed: false,
            child_vmar_s: BTreeMap::new(),
            vm_mappings: BTreeMap::new(),
            free_regions: BTreeMap::new(),
        }
    }

    /// Finds a free region for child `Vmar` or `VmMapping`.
    /// Returns (region base addr, child real offset).
    fn find_free_region(
        &mut self,
        child_offset: Option<Vaddr>,
        child_size: usize,
        align: usize,
    ) -> Result<(Vaddr, Vaddr)> {
        if let Some(child_vmar_offset) = child_offset {
            // if the offset is set, we should find a free region can satisfy both the offset and size
            let child_vmar_range = child_vmar_offset..(child_vmar_offset + child_size);
            for free_region in self.free_regions.find(&child_vmar_range) {
                let free_region_range = free_region.range();
                if free_region_range.start <= child_vmar_range.start
                    && child_vmar_range.end <= free_region_range.end
                {
                    return Ok((free_region_range.start, child_vmar_offset));
                }
            }
        } else {
            // Else, we find a free region that can satisfy the length and align requirement.
            // Here, we use a simple brute-force algorithm to find the first free range that can satisfy.
            // FIXME: A randomized algorithm may be more efficient.
            for (region_base, free_region) in &self.free_regions {
                let region_start = free_region.start();
                let region_end = free_region.end();
                let child_vmar_real_start = region_start.align_up(align);
                let child_vmar_real_end =
                    child_vmar_real_start
                        .checked_add(child_size)
                        .ok_or(Error::with_message(
                            Errno::ENOMEM,
                            "integer overflow when (child_vmar_real_start + child_size)",
                        ))?;
                if region_start <= child_vmar_real_start && child_vmar_real_end <= region_end {
                    return Ok((*region_base, child_vmar_real_start));
                }
            }
        }
        return_errno_with_message!(Errno::EACCES, "Cannot find free region for child")
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
    fn new(
        inner: VmarInner,
        vm_space: Arc<VmSpace>,
        base: usize,
        size: usize,
        parent: Option<&Arc<Vmar_>>,
    ) -> Arc<Self> {
        let parent = if let Some(parent) = parent {
            Arc::downgrade(parent)
        } else {
            Weak::new()
        };

        Arc::new(Vmar_ {
            inner: RwMutex::new(inner),
            base,
            size,
            vm_space,
            parent,
        })
    }

    fn new_root() -> Arc<Self> {
        let mut free_regions = BTreeMap::new();
        let root_region = FreeRegion::new(ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_CAP_ADDR);
        free_regions.insert(root_region.start(), root_region);
        let vmar_inner = VmarInner {
            is_destroyed: false,
            child_vmar_s: BTreeMap::new(),
            vm_mappings: BTreeMap::new(),
            free_regions,
        };
        let mut vm_space = VmSpace::new();
        vm_space.register_page_fault_handler(handle_page_fault_wrapper);
        Vmar_::new(vmar_inner, Arc::new(vm_space), 0, ROOT_VMAR_CAP_ADDR, None)
    }

    fn is_root_vmar(&self) -> bool {
        self.parent.upgrade().is_none()
    }

    fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        assert!(range.start % PAGE_SIZE == 0);
        assert!(range.end % PAGE_SIZE == 0);
        self.ensure_range_mapped(&range)?;
        self.do_protect_inner(perms, range)?;
        Ok(())
    }

    // Do real protect. The protected range is ensured to be mapped.
    fn do_protect_inner(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        let protect_mappings: Vec<Arc<VmMapping>> = {
            let inner = self.inner.read();
            inner
                .vm_mappings
                .find(&range)
                .into_iter()
                .cloned()
                .collect()
        };

        for vm_mapping in protect_mappings {
            let vm_mapping_range =
                vm_mapping.map_to_addr()..(vm_mapping.map_to_addr() + vm_mapping.map_size());
            let intersected_range = get_intersected_range(&range, &vm_mapping_range);
            vm_mapping.protect(perms, intersected_range)?;
        }

        for child_vmar_ in self.inner.read().child_vmar_s.find(&range) {
            let child_vmar_range = child_vmar_.range();
            debug_assert!(is_intersected(&child_vmar_range, &range));
            let intersected_range = get_intersected_range(&range, &child_vmar_range);
            child_vmar_.do_protect_inner(perms, intersected_range)?;
        }

        Ok(())
    }

    /// Ensure the whole protected range is mapped.
    /// Internally, we check whether the range intersects any free region recursively.
    /// If so, the range is not fully mapped.
    fn ensure_range_mapped(&self, range: &Range<usize>) -> Result<()> {
        // The protected range should be in self's range
        assert!(self.base <= range.start);
        assert!(range.end <= self.base + self.size);

        // The protected range should not intersect with any free region
        let inner = self.inner.read();
        if inner.free_regions.find(range).into_iter().next().is_some() {
            return_errno_with_message!(Errno::EACCES, "protected range is not fully mapped");
        }

        // if the protected range intersects with child `Vmar_`, child `Vmar_` is responsible to do the check.
        for child_vmar_ in inner.child_vmar_s.find(range) {
            let child_vmar_range = child_vmar_.range();
            debug_assert!(is_intersected(&child_vmar_range, range));
            let intersected_range = get_intersected_range(range, &child_vmar_range);
            child_vmar_.ensure_range_mapped(&intersected_range)?;
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
        if let Some(child_vmar) = inner.child_vmar_s.find_one(&address) {
            debug_assert!(child_vmar.range().contains(&address));
            return child_vmar.handle_page_fault(page_fault_info);
        }

        // FIXME: If multiple VMOs are mapped to the addr, should we allow all VMOs to handle page fault?
        if let Some(vm_mapping) = inner.vm_mappings.find_one(&address) {
            debug_assert!(vm_mapping.range().contains(&address));
            return vm_mapping.handle_page_fault(page_fault_info);
        }

        return_errno_with_message!(Errno::EACCES, "page fault addr is not in current vmar");
    }

    /// Clears all content of the root VMAR.
    fn clear_root_vmar(&self) -> Result<()> {
        debug_assert!(self.is_root_vmar());
        if !self.is_root_vmar() {
            return_errno_with_message!(Errno::EACCES, "The vmar is not root vmar");
        }
        self.clear_vm_space();
        let mut inner = self.inner.write();
        inner.child_vmar_s.clear();
        inner.vm_mappings.clear();
        inner.free_regions.clear();
        let root_region = FreeRegion::new(ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_CAP_ADDR);
        inner.free_regions.insert(root_region.start(), root_region);
        Ok(())
    }

    fn clear_vm_space(&self) {
        self.vm_space
            .cursor_mut_with(&(0..ROOT_VMAR_CAP_ADDR), |cursor| {
                cursor.unmap(ROOT_VMAR_CAP_ADDR);
            })
            .unwrap();
    }

    pub fn destroy(&self, range: Range<usize>) -> Result<()> {
        self.check_destroy_range(&range)?;
        let mut inner = self.inner.write();
        let mut free_regions = BTreeMap::new();

        for child_vmar_ in inner.child_vmar_s.find(&range) {
            let child_vmar_range = child_vmar_.range();
            debug_assert!(is_intersected(&child_vmar_range, &range));
            let free_region = FreeRegion::new(child_vmar_range);
            free_regions.insert(free_region.start(), free_region);
        }

        inner
            .child_vmar_s
            .retain(|_, child_vmar_| !child_vmar_.is_destroyed());

        let mut mappings_to_remove = LinkedList::new();
        let mut mappings_to_append = LinkedList::new();

        for vm_mapping in inner.vm_mappings.find(&range) {
            let vm_mapping_range = vm_mapping.range();
            debug_assert!(is_intersected(&vm_mapping_range, &range));
            let intersected_range = get_intersected_range(&vm_mapping_range, &range);
            vm_mapping.trim_mapping(
                &intersected_range,
                &mut mappings_to_remove,
                &mut mappings_to_append,
            )?;
            let free_region = FreeRegion::new(intersected_range);
            free_regions.insert(free_region.start(), free_region);
        }

        for mapping in mappings_to_remove {
            inner.vm_mappings.remove(&mapping);
        }
        for (map_to_addr, mapping) in mappings_to_append {
            inner.vm_mappings.insert(map_to_addr, mapping);
        }

        inner
            .vm_mappings
            .retain(|_, vm_mapping| !vm_mapping.is_destroyed());
        inner.free_regions.append(&mut free_regions);
        drop(inner);
        self.merge_continuous_regions();
        Ok(())
    }

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
        self.ensure_range_mapped(&(map_addr..old_map_end))?;

        if new_size < old_size {
            self.destroy(new_map_end..old_map_end)?;
            return Ok(());
        }

        let last_mapping = {
            let inner = self.inner.read();
            inner
                .vm_mappings
                .find_one(&(old_map_end - 1))
                .unwrap()
                .clone()
        };

        let extra_mapping_start = last_mapping.map_end();
        let free_region = self.allocate_free_region_for_mapping(
            new_map_end - extra_mapping_start,
            Some(extra_mapping_start),
            PAGE_SIZE,
            false,
        )?;
        last_mapping.enlarge(new_map_end - extra_mapping_start);
        Ok(())
    }

    fn check_destroy_range(&self, range: &Range<usize>) -> Result<()> {
        debug_assert!(range.start % PAGE_SIZE == 0);
        debug_assert!(range.end % PAGE_SIZE == 0);

        let inner = self.inner.read();

        for child_vmar_ in inner.child_vmar_s.find(range) {
            let child_vmar_range = child_vmar_.range();
            debug_assert!(is_intersected(&child_vmar_range, range));
            if range.start <= child_vmar_range.start && child_vmar_range.end <= range.end {
                // Child vmar is totally in the range.
                continue;
            }
            return_errno_with_message!(
                Errno::EACCES,
                "Child vmar is partly intersected with destroyed range"
            );
        }

        Ok(())
    }

    fn is_destroyed(&self) -> bool {
        self.inner.read().is_destroyed
    }

    fn merge_continuous_regions(&self) {
        let mut new_free_regions = BTreeMap::new();
        let mut inner = self.inner.write();
        let keys = inner.free_regions.keys().cloned().collect::<Vec<_>>();
        for key in keys {
            if let Some(mut free_region) = inner.free_regions.remove(&key) {
                let mut region_end = free_region.end();
                while let Some(another_region) = inner.free_regions.remove(&region_end) {
                    free_region.merge_other_region(&another_region);
                    region_end = another_region.end();
                }
                new_free_regions.insert(free_region.start(), free_region);
            }
        }
        inner.free_regions.clear();
        inner.free_regions.append(&mut new_free_regions);
    }

    /// Allocate a child `Vmar_`.
    pub fn alloc_child_vmar(
        self: &Arc<Self>,
        child_vmar_offset: Option<usize>,
        child_vmar_size: usize,
        align: usize,
    ) -> Result<Arc<Vmar_>> {
        let (region_base, child_vmar_offset) =
            self.inner
                .write()
                .find_free_region(child_vmar_offset, child_vmar_size, align)?;
        // This unwrap should never fails
        let free_region = self
            .inner
            .write()
            .free_regions
            .remove(&region_base)
            .unwrap();
        let child_range = child_vmar_offset..(child_vmar_offset + child_vmar_size);
        let regions_after_allocation = free_region.allocate_range(child_range.clone());
        regions_after_allocation.into_iter().for_each(|region| {
            self.inner
                .write()
                .free_regions
                .insert(region.start(), region);
        });
        let child_region = FreeRegion::new(child_range);
        let mut child_regions = BTreeMap::new();
        child_regions.insert(child_region.start(), child_region);
        let child_vmar_inner = VmarInner {
            is_destroyed: false,
            child_vmar_s: BTreeMap::new(),
            vm_mappings: BTreeMap::new(),
            free_regions: child_regions,
        };
        let child_vmar_ = Vmar_::new(
            child_vmar_inner,
            self.vm_space.clone(),
            child_vmar_offset,
            child_vmar_size,
            Some(self),
        );
        self.inner
            .write()
            .child_vmar_s
            .insert(child_vmar_.base, child_vmar_.clone());
        Ok(child_vmar_)
    }

    fn check_overwrite(&self, mapping_range: Range<usize>, can_overwrite: bool) -> Result<()> {
        let inner = self.inner.read();
        if inner
            .child_vmar_s
            .find(&mapping_range)
            .into_iter()
            .next()
            .is_some()
        {
            return_errno_with_message!(
                Errno::EACCES,
                "mapping range overlapped with child vmar range"
            );
        }

        if !can_overwrite
            && inner
                .vm_mappings
                .find(&mapping_range)
                .into_iter()
                .next()
                .is_some()
        {
            return_errno_with_message!(
                Errno::EACCES,
                "mapping range overlapped with another mapping"
            );
        }

        Ok(())
    }

    /// Returns the attached `VmSpace`.
    fn vm_space(&self) -> &Arc<VmSpace> {
        &self.vm_space
    }

    /// Maps a `VmMapping` to this VMAR.
    fn add_mapping(&self, mapping: Arc<VmMapping>) {
        self.inner
            .write()
            .vm_mappings
            .insert(mapping.map_to_addr(), mapping);
    }

    fn allocate_free_region_for_mapping(
        &self,
        map_size: usize,
        offset: Option<usize>,
        align: usize,
        can_overwrite: bool,
    ) -> Result<Vaddr> {
        trace!("allocate free region, map_size = 0x{:x}, offset = {:x?}, align = 0x{:x}, can_overwrite = {}", map_size, offset, align, can_overwrite);

        if can_overwrite {
            let mut inner = self.inner.write();
            // If can overwrite, the offset is ensured not to be `None`.
            let offset = offset.ok_or(Error::with_message(
                Errno::EINVAL,
                "offset cannot be None since can overwrite is set",
            ))?;
            let map_range = offset..(offset + map_size);
            // If can overwrite, the mapping can cross multiple free regions. We will split each free regions that intersect with the mapping.
            let mut split_regions = Vec::new();

            for free_region in inner.free_regions.find(&map_range) {
                let free_region_range = free_region.range();
                if is_intersected(&free_region_range, &map_range) {
                    split_regions.push(free_region_range.start);
                }
            }

            for region_base in split_regions {
                let free_region = inner.free_regions.remove(&region_base).unwrap();
                let intersected_range = get_intersected_range(&free_region.range(), &map_range);
                let regions_after_split = free_region.allocate_range(intersected_range);
                regions_after_split.into_iter().for_each(|region| {
                    inner.free_regions.insert(region.start(), region);
                });
            }
            drop(inner);
            self.trim_existing_mappings(map_range)?;
            Ok(offset)
        } else {
            // Otherwise, the mapping in a single region.
            let mut inner = self.inner.write();
            let (free_region_base, offset) = inner.find_free_region(offset, map_size, align)?;
            let free_region = inner.free_regions.remove(&free_region_base).unwrap();
            let mapping_range = offset..(offset + map_size);
            let intersected_range = get_intersected_range(&free_region.range(), &mapping_range);
            let regions_after_split = free_region.allocate_range(intersected_range);
            regions_after_split.into_iter().for_each(|region| {
                inner.free_regions.insert(region.start(), region);
            });
            Ok(offset)
        }
    }

    fn trim_existing_mappings(&self, trim_range: Range<usize>) -> Result<()> {
        let mut inner = self.inner.write();
        let mut mappings_to_remove = LinkedList::new();
        let mut mappings_to_append = LinkedList::new();
        for vm_mapping in inner.vm_mappings.find(&trim_range) {
            vm_mapping.trim_mapping(
                &trim_range,
                &mut mappings_to_remove,
                &mut mappings_to_append,
            )?;
        }

        for map_addr in mappings_to_remove {
            inner.vm_mappings.remove(&map_addr);
        }
        for (map_addr, mapping) in mappings_to_append {
            inner.vm_mappings.insert(map_addr, mapping);
        }
        Ok(())
    }

    pub(super) fn new_fork_root(self: &Arc<Self>) -> Result<Arc<Self>> {
        if self.parent.upgrade().is_some() {
            return_errno_with_message!(Errno::EINVAL, "can only dup cow vmar for root vmar");
        }

        self.new_fork(None)
    }

    /// Creates a new fork VMAR with Copy-On-Write (COW) mechanism.
    fn new_fork(&self, parent: Option<&Arc<Vmar_>>) -> Result<Arc<Self>> {
        let new_vmar_ = {
            let vmar_inner = VmarInner::new();
            // If this is not a root `Vmar`, we clone the `VmSpace` from parent.
            //
            // If this is a root `Vmar`, we leverage Copy-On-Write (COW) mechanism to
            // clone the `VmSpace` to the child.
            let vm_space = if let Some(parent) = parent {
                parent.vm_space().clone()
            } else {
                let mut new_space = VmSpace::new();
                new_space.register_page_fault_handler(handle_page_fault_wrapper);
                Arc::new(new_space)
            };
            Vmar_::new(vmar_inner, vm_space, self.base, self.size, parent)
        };

        let inner = self.inner.read();
        let mut new_inner = new_vmar_.inner.write();

        // Clone free regions.
        for (free_region_base, free_region) in &inner.free_regions {
            new_inner
                .free_regions
                .insert(*free_region_base, free_region.clone());
        }

        // Clone child vmars.
        for (child_vmar_base, child_vmar_) in &inner.child_vmar_s {
            let new_child_vmar = child_vmar_.new_fork(Some(&new_vmar_))?;
            new_inner
                .child_vmar_s
                .insert(*child_vmar_base, new_child_vmar);
        }

        // Clone mappings.
        {
            let new_vmspace = new_vmar_.vm_space();
            let range = self.base..(self.base + self.size);
            let cur_vmspace = self.vm_space();
            cur_vmspace
                .cursor_mut_with(&range, |cur_cursor| {
                    new_vmspace
                        .cursor_mut_with(&range, |new_cursor| {
                            for (vm_mapping_base, vm_mapping) in &inner.vm_mappings {
                                // Clone the `VmMapping` to the new VMAR.
                                let new_mapping = Arc::new(vm_mapping.new_fork(&new_vmar_)?);
                                new_inner.vm_mappings.insert(*vm_mapping_base, new_mapping);

                                // Protect the mapping and copy to the new page table for COW.
                                cur_cursor.jump(*vm_mapping_base).unwrap();
                                new_cursor.jump(*vm_mapping_base).unwrap();
                                let mut op = |page: &mut PageProperty| {
                                    page.flags -= PageFlags::W;
                                };
                                new_cursor.copy_from(cur_cursor, vm_mapping.map_size(), &mut op);
                            }
                            cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::All);
                            cur_cursor.flusher().dispatch_tlb_flush();

                            Result::Ok(())
                        })
                        .unwrap()?;
                    Result::Ok(())
                })
                .unwrap()?;
        }

        drop(new_inner);

        Ok(new_vmar_)
    }
}

/// This is for fallible user space write handling.
fn handle_page_fault_wrapper(
    vm_space: &VmSpace,
    trap_info: &CpuExceptionInfo,
) -> core::result::Result<(), ()> {
    handle_page_fault_from_vm_space(vm_space, &trap_info.try_into().unwrap())
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

#[derive(Debug, Clone)]
pub struct FreeRegion {
    range: Range<Vaddr>,
}

impl Interval<usize> for FreeRegion {
    fn range(&self) -> Range<usize> {
        self.range.clone()
    }
}

impl FreeRegion {
    pub fn new(range: Range<Vaddr>) -> Self {
        Self { range }
    }

    pub fn start(&self) -> Vaddr {
        self.range.start
    }

    pub fn end(&self) -> Vaddr {
        self.range.end
    }

    pub fn size(&self) -> usize {
        self.range.end - self.range.start
    }

    /// Allocates a range in this free region.
    ///
    /// The range is ensured to be contained in current region before call this function.
    /// The return vector contains regions that are not allocated. Since the `allocate_range` can be
    /// in the middle of a free region, the original region may be split as at most two regions.
    pub fn allocate_range(&self, allocate_range: Range<Vaddr>) -> Vec<FreeRegion> {
        let mut res = Vec::new();
        if self.range.start < allocate_range.start {
            let free_region = FreeRegion::new(self.range.start..allocate_range.start);
            res.push(free_region);
        }
        if allocate_range.end < self.range.end {
            let free_region = FreeRegion::new(allocate_range.end..self.range.end);
            res.push(free_region);
        }
        res
    }

    pub fn merge_other_region(&mut self, other_region: &FreeRegion) {
        assert!(self.range.end == other_region.range.start);
        assert!(self.range.start < other_region.range.end);
        self.range = self.range.start..other_region.range.end
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

impl<'a, V: Interval<Vaddr> + 'a> IntervalSet<'a, Vaddr> for BTreeMap<Vaddr, V> {
    type Item = V;
    fn find(&'a self, range: &Range<Vaddr>) -> impl IntoIterator<Item = &'a Self::Item> + 'a {
        let mut res = Vec::new();
        let mut cursor = self.lower_bound(core::ops::Bound::Excluded(&range.start));
        // There's one previous element that may intersect with the range.
        if let Some((_, v)) = cursor.peek_prev() {
            if v.range().end > range.start {
                res.push(v);
            }
        }
        // Find all intersected elements following it.
        while let Some((_, v)) = cursor.next() {
            if v.range().start >= range.end {
                break;
            }
            res.push(v);
        }

        res
    }

    fn find_one(&'a self, point: &Vaddr) -> Option<&'a Self::Item> {
        let cursor = self.lower_bound(core::ops::Bound::Excluded(point));
        // There's one previous element and one following element that may
        // contain the point. If they don't, there's no other chances.
        if let Some((_, v)) = cursor.peek_prev() {
            if v.range().end > *point {
                return Some(v);
            }
        } else if let Some((_, v)) = cursor.peek_next() {
            if v.range().start <= *point {
                return Some(v);
            }
        }
        None
    }
}
