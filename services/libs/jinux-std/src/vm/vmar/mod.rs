//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod options;
mod static_cap;
pub mod vm_mapping;

use crate::prelude::*;
use crate::vm::perms::VmPerms;
use align_ext::AlignExt;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::sync::Weak;
use alloc::vec::Vec;
use core::ops::Range;
use jinux_frame::vm::VmSpace;
use jinux_rights::Rights;

use self::vm_mapping::VmMapping;

use super::page_fault_handler::PageFaultHandler;

/// Virtual Memory Address Regions (VMARs) are a type of capability that manages
/// user address spaces.
///
/// # Capabilities
///
/// As a capability, each VMAR is associated with a set of access rights,
/// whose semantics are explained below.
///
/// The semantics of each access rights for VMARs are described below:
/// * The Dup right allows duplicating a VMAR and creating children out of
/// a VMAR.
/// * The Read, Write, Exec rights allow creating memory mappings with
/// readable, writable, and executable access permissions, respectively.
/// * The Read and Write rights allow the VMAR to be read from and written to
/// directly.
///
/// VMARs are implemented with two flavors of capabilities:
/// the dynamic one (`Vmar<Rights>`) and the static one (`Vmar<R: TRights>).
///
/// # Implementation
///
/// `Vmar` provides high-level APIs for address space management by wrapping
/// around its low-level counterpart `_frame::vm::VmFrames`.
/// Compared with `VmFrames`,
/// `Vmar` is easier to use (by offering more powerful APIs) and
/// harder to misuse (thanks to its nature of being capability).
///
pub struct Vmar<R = Rights>(Arc<Vmar_>, R);

pub trait VmarRightsOp {
    /// Returns the access rights.
    fn rights(&self) -> Rights;
    fn check_rights(&self, rights: Rights) -> Result<()>;
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
    default fn handle_page_fault(
        &self,
        page_fault_addr: Vaddr,
        not_present: bool,
        write: bool,
    ) -> Result<()> {
        unimplemented!()
    }
}

impl<R> Vmar<R> {
    /// FIXME: This function should require access control
    pub fn vm_space(&self) -> &VmSpace {
        self.0.vm_space()
    }
}

pub(super) struct Vmar_ {
    /// vmar inner
    inner: Mutex<VmarInner>,
    /// The offset relative to the root VMAR
    base: Vaddr,
    /// The total size of the VMAR in bytes
    size: usize,
    /// The attached vmspace
    vm_space: VmSpace,
    /// The parent vmar. If points to none, this is a root vmar
    parent: Weak<Vmar_>,
}

struct VmarInner {
    /// Whether the vmar is destroyed
    is_destroyed: bool,
    /// The child vmars. The key is offset relative to root VMAR
    child_vmar_s: BTreeMap<Vaddr, Arc<Vmar_>>,
    /// The mapped vmos. The key is offset relative to root VMAR
    vm_mappings: BTreeMap<Vaddr, Arc<VmMapping>>,
    /// Free regions that can be used for creating child vmar or mapping vmos
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
}

// FIXME: How to set the correct root vmar range?
// We should not include addr 0 here(is this right?), since the 0 addr means the null pointer.
// We should include addr 0x0040_0000, since non-pie executables typically are put on 0x0040_0000.
const ROOT_VMAR_LOWEST_ADDR: Vaddr = 0x0010_0000;
const ROOT_VMAR_HIGHEST_ADDR: Vaddr = 0x1000_0000_0000;

impl Vmar_ {
    fn new(
        inner: VmarInner,
        vm_space: VmSpace,
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
            inner: Mutex::new(inner),
            base,
            size,
            vm_space,
            parent,
        })
    }

    pub fn new_root() -> Arc<Self> {
        let mut free_regions = BTreeMap::new();
        let root_region = FreeRegion::new(ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_HIGHEST_ADDR);
        free_regions.insert(root_region.start(), root_region);
        let vmar_inner = VmarInner {
            is_destroyed: false,
            child_vmar_s: BTreeMap::new(),
            vm_mappings: BTreeMap::new(),
            free_regions,
        };
        Vmar_::new(vmar_inner, VmSpace::new(), 0, ROOT_VMAR_HIGHEST_ADDR, None)
    }

    fn is_root_vmar(&self) -> bool {
        self.parent.upgrade().is_none()
    }

    pub fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        assert!(range.start % PAGE_SIZE == 0);
        assert!(range.end % PAGE_SIZE == 0);
        self.check_protected_range(&range)?;
        self.do_protect_inner(perms, range)?;
        Ok(())
    }

    // do real protect. The protected range is ensured to be mapped.
    fn do_protect_inner(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        for (vm_mapping_base, vm_mapping) in &self.inner.lock().vm_mappings {
            let vm_mapping_range = *vm_mapping_base..(*vm_mapping_base + vm_mapping.map_size());
            if is_intersected(&range, &vm_mapping_range) {
                let intersected_range = get_intersected_range(&range, &vm_mapping_range);
                vm_mapping.protect(perms, intersected_range)?;
            }
        }

        for child_vmar_ in self.inner.lock().child_vmar_s.values() {
            let child_vmar_range = child_vmar_.range();
            if is_intersected(&range, &child_vmar_range) {
                let intersected_range = get_intersected_range(&range, &child_vmar_range);
                child_vmar_.do_protect_inner(perms, intersected_range)?;
            }
        }

        Ok(())
    }

    /// ensure the whole protected range is mapped, that is to say, backed up by a VMO.
    /// Internally, we check whether the range intersects any free region recursively.
    /// If so, the range is not fully mapped.
    fn check_protected_range(&self, protected_range: &Range<usize>) -> Result<()> {
        // The protected range should be in self's range
        assert!(self.base <= protected_range.start);
        assert!(protected_range.end <= self.base + self.size);

        // The protected range should not interstect with any free region
        for free_region in self.inner.lock().free_regions.values() {
            if is_intersected(&free_region.range, protected_range) {
                return_errno_with_message!(Errno::EACCES, "protected range is not fully mapped");
            }
        }

        // if the protected range intersects with child vmar_, child vmar_ is responsible to do the check.
        for child_vmar_ in self.inner.lock().child_vmar_s.values() {
            let child_range = child_vmar_.range();
            if is_intersected(&child_range, protected_range) {
                let intersected_range = get_intersected_range(&child_range, protected_range);
                child_vmar_.check_protected_range(&intersected_range)?;
            }
        }

        Ok(())
    }

    /// Handle user space page fault, if the page fault is successfully handled ,return Ok(()).
    pub fn handle_page_fault(
        &self,
        page_fault_addr: Vaddr,
        not_present: bool,
        write: bool,
    ) -> Result<()> {
        if page_fault_addr < self.base || page_fault_addr >= self.base + self.size {
            return_errno_with_message!(Errno::EACCES, "page fault addr is not in current vmar");
        }

        let inner = self.inner.lock();
        for (child_vmar_base, child_vmar) in &inner.child_vmar_s {
            if *child_vmar_base <= page_fault_addr
                && page_fault_addr < *child_vmar_base + child_vmar.size
            {
                return child_vmar.handle_page_fault(page_fault_addr, not_present, write);
            }
        }

        // FIXME: If multiple vmos are mapped to the addr, should we allow all vmos to handle page fault?
        for (vm_mapping_base, vm_mapping) in &inner.vm_mappings {
            if *vm_mapping_base <= page_fault_addr
                && page_fault_addr < *vm_mapping_base + vm_mapping.map_size()
            {
                return vm_mapping.handle_page_fault(page_fault_addr, not_present, write);
            }
        }

        return_errno_with_message!(Errno::EACCES, "page fault addr is not in current vmar");
    }

    /// clear all content of the root vmar
    pub fn clear_root_vmar(&self) -> Result<()> {
        debug_assert!(self.is_root_vmar());
        if !self.is_root_vmar() {
            return_errno_with_message!(Errno::EACCES, "The vmar is not root vmar");
        }
        self.vm_space.clear();
        let mut inner = self.inner.lock();
        inner.child_vmar_s.clear();
        inner.vm_mappings.clear();
        inner.free_regions.clear();
        let root_region = FreeRegion::new(ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_HIGHEST_ADDR);
        inner.free_regions.insert(root_region.start(), root_region);
        Ok(())
    }

    pub fn destroy_all(&self) -> Result<()> {
        let mut inner = self.inner.lock();
        inner.is_destroyed = true;
        let mut free_regions = BTreeMap::new();
        for (child_vmar_base, child_vmar) in &inner.child_vmar_s {
            child_vmar.destroy_all()?;
            let free_region = FreeRegion::new(child_vmar.range());
            free_regions.insert(free_region.start(), free_region);
        }
        inner.child_vmar_s.clear();
        inner.free_regions.append(&mut free_regions);

        for vm_mapping in inner.vm_mappings.values() {
            vm_mapping.unmap(&vm_mapping.range(), true)?;
            let free_region = FreeRegion::new(vm_mapping.range());
            free_regions.insert(free_region.start(), free_region);
        }
        inner.vm_mappings.clear();
        inner.free_regions.append(&mut free_regions);

        drop(inner);
        self.merge_continuous_regions();
        self.vm_space.clear();
        Ok(())
    }

    pub fn destroy(&self, range: Range<usize>) -> Result<()> {
        self.check_destroy_range(&range)?;
        let mut inner = self.inner.lock();
        let mut free_regions = BTreeMap::new();
        for (child_vmar_base, child_vmar) in &inner.child_vmar_s {
            let child_vmar_range = child_vmar.range();
            if is_intersected(&range, &child_vmar_range) {
                child_vmar.destroy_all()?;
            }
            let free_region = FreeRegion::new(child_vmar_range);
            free_regions.insert(free_region.start(), free_region);
        }

        inner
            .child_vmar_s
            .retain(|_, child_vmar_| !child_vmar_.is_destroyed());

        let mut mappings_to_remove = BTreeSet::new();
        let mut mappings_to_append = BTreeMap::new();
        for vm_mapping in inner.vm_mappings.values() {
            let vm_mapping_range = vm_mapping.range();
            if is_intersected(&vm_mapping_range, &range) {
                let intersected_range = get_intersected_range(&vm_mapping_range, &range);
                vm_mapping.trim_mapping(
                    &intersected_range,
                    &mut mappings_to_remove,
                    &mut mappings_to_append,
                )?;
                let free_region = FreeRegion::new(intersected_range);
                free_regions.insert(free_region.start(), free_region);
            }
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

    fn check_destroy_range(&self, range: &Range<usize>) -> Result<()> {
        debug_assert!(range.start % PAGE_SIZE == 0);
        debug_assert!(range.end % PAGE_SIZE == 0);

        for (child_vmar_base, child_vmar) in &self.inner.lock().child_vmar_s {
            let child_vmar_start = *child_vmar_base;
            let child_vmar_end = child_vmar_start + child_vmar.size;
            if child_vmar_end <= range.start || child_vmar_start >= range.end {
                // child vmar does not intersect with range
                continue;
            }
            if range.start <= child_vmar_start && child_vmar_end <= range.end {
                // child vmar is totolly in the range
                continue;
            }
            assert!(is_intersected(range, &(child_vmar_start..child_vmar_end)));
            return_errno_with_message!(
                Errno::EACCES,
                "Child vmar is partly intersected with destryed range"
            );
        }

        Ok(())
    }

    fn is_destroyed(&self) -> bool {
        self.inner.lock().is_destroyed
    }

    fn merge_continuous_regions(&self) {
        let mut new_free_regions = BTreeMap::new();
        let mut inner = self.inner.lock();
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

    pub fn read(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let read_start = self.base + offset;
        let read_end = buf.len() + read_start;
        // if the read range is in child vmar
        for (child_vmar_base, child_vmar) in &self.inner.lock().child_vmar_s {
            let child_vmar_end = *child_vmar_base + child_vmar.size;
            if *child_vmar_base <= read_start && read_end <= child_vmar_end {
                let child_offset = read_start - *child_vmar_base;
                return child_vmar.read(child_offset, buf);
            }
        }
        // if the read range is in mapped vmo
        for (vm_mapping_base, vm_mapping) in &self.inner.lock().vm_mappings {
            let vm_mapping_end = *vm_mapping_base + vm_mapping.map_size();
            if *vm_mapping_base <= read_start && read_end <= vm_mapping_end {
                let vm_mapping_offset = read_start - *vm_mapping_base;
                return vm_mapping.read_bytes(vm_mapping_offset, buf);
            }
        }

        // FIXME: If the read range is across different vmos or child vmars, should we directly return error?
        return_errno_with_message!(Errno::EACCES, "read range is not backed up by a vmo");
    }

    pub fn write(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let write_start = self
            .base
            .checked_add(offset)
            .ok_or_else(|| Error::with_message(Errno::EFAULT, "Arithmetic Overflow"))?;

        let write_end = buf
            .len()
            .checked_add(write_start)
            .ok_or_else(|| Error::with_message(Errno::EFAULT, "Arithmetic Overflow"))?;

        // if the write range is in child vmar
        for (child_vmar_base, child_vmar) in &self.inner.lock().child_vmar_s {
            let child_vmar_end = *child_vmar_base + child_vmar.size;
            if *child_vmar_base <= write_start && write_end <= child_vmar_end {
                let child_offset = write_start - *child_vmar_base;
                return child_vmar.write(child_offset, buf);
            }
        }
        // if the write range is in mapped vmo
        for (vm_mapping_base, vm_mapping) in &self.inner.lock().vm_mappings {
            let vm_mapping_end = *vm_mapping_base + vm_mapping.map_size();
            if *vm_mapping_base <= write_start && write_end <= vm_mapping_end {
                let vm_mapping_offset = write_start - *vm_mapping_base;
                return vm_mapping.write_bytes(vm_mapping_offset, buf);
            }
        }

        // FIXME: If the write range is across different vmos or child vmars, should we directly return error?
        return_errno_with_message!(Errno::EACCES, "write range is not backed up by a vmo");
    }

    /// allocate a child vmar_.
    pub fn alloc_child_vmar(
        self: &Arc<Self>,
        child_vmar_offset: Option<usize>,
        child_vmar_size: usize,
        align: usize,
    ) -> Result<Arc<Vmar_>> {
        let (region_base, child_vmar_offset) =
            self.find_free_region_for_child(child_vmar_offset, child_vmar_size, align)?;
        // This unwrap should never fails
        let free_region = self.inner.lock().free_regions.remove(&region_base).unwrap();
        let child_range = child_vmar_offset..(child_vmar_offset + child_vmar_size);
        let regions_after_allocation = free_region.allocate_range(child_range.clone());
        regions_after_allocation.into_iter().for_each(|region| {
            self.inner
                .lock()
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
            .lock()
            .child_vmar_s
            .insert(child_vmar_.base, child_vmar_.clone());
        Ok(child_vmar_)
    }

    /// find a free region for child vmar or vmo.
    /// returns (region base addr, child real offset)
    fn find_free_region_for_child(
        &self,
        child_offset: Option<Vaddr>,
        child_size: usize,
        align: usize,
    ) -> Result<(Vaddr, Vaddr)> {
        for (region_base, free_region) in &self.inner.lock().free_regions {
            if let Some(child_vmar_offset) = child_offset {
                // if the offset is set, we should find a free region can satisfy both the offset and size
                if *region_base <= child_vmar_offset
                    && (child_vmar_offset + child_size) <= (free_region.end())
                {
                    return Ok((*region_base, child_vmar_offset));
                }
            } else {
                // else, we find a free region that can satisfy the length and align requirement.
                // Here, we use a simple brute-force algorithm to find the first free range that can satisfy.
                // FIXME: A randomized algorithm may be more efficient.
                let region_start = free_region.start();
                let region_end = free_region.end();
                let child_vmar_real_start = region_start.align_up(align);
                let child_vmar_real_end = child_vmar_real_start + child_size;
                if region_start <= child_vmar_real_start && child_vmar_real_end <= region_end {
                    return Ok((*region_base, child_vmar_real_start));
                }
            }
        }
        return_errno_with_message!(Errno::EACCES, "Cannot find free region for child")
    }

    fn range(&self) -> Range<usize> {
        self.base..(self.base + self.size)
    }

    fn check_vmo_overwrite(&self, vmo_range: Range<usize>, can_overwrite: bool) -> Result<()> {
        let inner = self.inner.lock();
        for child_vmar in inner.child_vmar_s.values() {
            let child_vmar_range = child_vmar.range();
            if is_intersected(&vmo_range, &child_vmar_range) {
                return_errno_with_message!(
                    Errno::EACCES,
                    "vmo range overlapped with child vmar range"
                );
            }
        }

        if !can_overwrite {
            for (child_vmo_base, child_vmo) in &inner.vm_mappings {
                let child_vmo_range = *child_vmo_base..*child_vmo_base + child_vmo.map_size();
                if is_intersected(&vmo_range, &child_vmo_range) {
                    debug!("vmo_range = {:x?}", vmo_range);
                    debug!("child_vmo_range = {:x?}", child_vmo_range);
                    return_errno_with_message!(
                        Errno::EACCES,
                        "vmo range overlapped with another vmo"
                    );
                }
            }
        }

        Ok(())
    }

    /// returns the attached vm_space
    pub(super) fn vm_space(&self) -> &VmSpace {
        &self.vm_space
    }

    /// map a vmo to this vmar
    pub fn add_mapping(&self, mapping: Arc<VmMapping>) {
        self.inner
            .lock()
            .vm_mappings
            .insert(mapping.map_to_addr(), mapping);
    }

    fn allocate_free_region_for_vmo(
        &self,
        vmo_size: usize,
        size: usize,
        offset: Option<usize>,
        align: usize,
        can_overwrite: bool,
    ) -> Result<Vaddr> {
        trace!("allocate free region, vmo_size = 0x{:x}, map_size = 0x{:x}, offset = {:x?}, align = 0x{:x}, can_ovewrite = {}", vmo_size, size, offset, align, can_overwrite);
        let map_size = size.max(vmo_size);

        if can_overwrite {
            let mut inner = self.inner.lock();
            // if can_overwrite, the offset is ensured not to be None
            let offset = offset.ok_or(Error::with_message(
                Errno::EINVAL,
                "offset cannot be None since can overwrite is set",
            ))?;
            let map_range = offset..(offset + map_size);
            // If can overwrite, the vmo can cross multiple free regions. We will split each free regions that intersect with the vmo
            let mut split_regions = Vec::new();
            for (free_region_base, free_region) in &inner.free_regions {
                let free_region_range = free_region.range();
                if is_intersected(free_region_range, &map_range) {
                    split_regions.push(*free_region_base);
                }
            }
            for region_base in split_regions {
                let free_region = inner.free_regions.remove(&region_base).unwrap();
                let intersected_range = get_intersected_range(free_region.range(), &map_range);
                let regions_after_split = free_region.allocate_range(intersected_range);
                regions_after_split.into_iter().for_each(|region| {
                    inner.free_regions.insert(region.start(), region);
                });
            }
            drop(inner);
            self.trim_existing_mappings(map_range)?;
            Ok(offset)
        } else {
            // Otherwise, the vmo in a single region
            let (free_region_base, offset) =
                self.find_free_region_for_child(offset, map_size, align)?;
            let mut inner = self.inner.lock();
            let free_region = inner.free_regions.remove(&free_region_base).unwrap();
            let vmo_range = offset..(offset + map_size);
            let intersected_range = get_intersected_range(free_region.range(), &vmo_range);
            let regions_after_split = free_region.allocate_range(intersected_range);
            regions_after_split.into_iter().for_each(|region| {
                inner.free_regions.insert(region.start(), region);
            });
            Ok(offset)
        }
    }

    fn trim_existing_mappings(&self, trim_range: Range<usize>) -> Result<()> {
        let mut inner = self.inner.lock();
        let mut mappings_to_remove = BTreeSet::new();
        let mut mappings_to_append = BTreeMap::new();
        for vm_mapping in inner.vm_mappings.values() {
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

    pub(super) fn new_cow_root(self: &Arc<Self>) -> Result<Arc<Self>> {
        if self.parent.upgrade().is_some() {
            return_errno_with_message!(Errno::EINVAL, "can only dup cow vmar for root vmar");
        }

        self.new_cow(None)
    }

    /// Create a new vmar by creating cow child for all mapped vmos
    fn new_cow(&self, parent: Option<&Arc<Vmar_>>) -> Result<Arc<Self>> {
        let new_vmar_ = {
            let vmar_inner = VmarInner::new();
            // If this is a root vmar, we create a new vmspace,
            // Otherwise, we clone the vm space from parent.
            let vm_space = if let Some(parent) = parent {
                parent.vm_space().clone()
            } else {
                VmSpace::new()
            };
            Vmar_::new(vmar_inner, vm_space, self.base, self.size, parent)
        };

        let inner = self.inner.lock();
        // clone free regions
        for (free_region_base, free_region) in &inner.free_regions {
            new_vmar_
                .inner
                .lock()
                .free_regions
                .insert(*free_region_base, free_region.clone());
        }

        // clone child vmars
        for (child_vmar_base, child_vmar_) in &inner.child_vmar_s {
            let new_child_vmar = child_vmar_.new_cow(Some(&new_vmar_))?;
            new_vmar_
                .inner
                .lock()
                .child_vmar_s
                .insert(*child_vmar_base, new_child_vmar);
        }

        // clone vm mappings
        for (vm_mapping_base, vm_mapping) in &inner.vm_mappings {
            let new_mapping = Arc::new(vm_mapping.new_cow(&new_vmar_)?);
            new_vmar_
                .inner
                .lock()
                .vm_mappings
                .insert(*vm_mapping_base, new_mapping);
        }
        Ok(new_vmar_)
    }

    /// get mapped vmo at given offset
    fn get_vm_mapping(&self, offset: Vaddr) -> Result<Arc<VmMapping>> {
        for (vm_mapping_base, vm_mapping) in &self.inner.lock().vm_mappings {
            if *vm_mapping_base <= offset && offset < *vm_mapping_base + vm_mapping.map_size() {
                return Ok(vm_mapping.clone());
            }
        }
        return_errno_with_message!(Errno::EACCES, "No mapped vmo at this offset");
    }
}

impl<R> Vmar<R> {
    /// The base address, i.e., the offset relative to the root VMAR.
    ///
    /// The base address of a root VMAR is zero.
    pub fn base(&self) -> Vaddr {
        self.0.base
    }

    /// The size of the vmar in bytes.
    pub fn size(&self) -> usize {
        self.0.size
    }

    /// get a mapped vmo
    pub fn get_vm_mapping(&self, offset: Vaddr) -> Result<Arc<VmMapping>> {
        let rights = Rights::all();
        self.check_rights(rights)?;
        self.0.get_vm_mapping(offset)
    }
}

#[derive(Debug, Clone)]
pub struct FreeRegion {
    range: Range<Vaddr>,
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

    pub fn range(&self) -> &Range<usize> {
        &self.range
    }

    /// allocate a range in this free region.
    /// The range is ensured to be contained in current region before call this function.
    /// The return vector contains regions that are not allocated. Since the allocate_range can be
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

/// determine whether two ranges are intersected.
pub fn is_intersected(range1: &Range<usize>, range2: &Range<usize>) -> bool {
    range1.start.max(range2.start) < range1.end.min(range2.end)
}

/// get the intersection range of two ranges.
/// The two ranges should be ensured to be intersected.
pub fn get_intersected_range(range1: &Range<usize>, range2: &Range<usize>) -> Range<usize> {
    debug_assert!(is_intersected(range1, range2));
    range1.start.max(range2.start)..range1.end.min(range2.end)
}
