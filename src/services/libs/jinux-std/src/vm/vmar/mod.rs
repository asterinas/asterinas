//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod options;
mod static_cap;
pub mod vm_mapping;

use crate::rights::Rights;
use crate::vm::perms::VmPerms;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::sync::Weak;
use alloc::vec::Vec;
use core::ops::Range;
use jinux_frame::config::PAGE_SIZE;
use jinux_frame::vm::Vaddr;
use jinux_frame::vm::VmSpace;
use jinux_frame::AlignExt;
use jinux_frame::{Error, Result};
use spin::Mutex;

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

// TODO: how page faults can be delivered to and handled by the current VMAR.
impl<R> PageFaultHandler for Vmar<R> {
    default fn handle_page_fault(&self, page_fault_addr: Vaddr, write: bool) -> Result<()> {
        unimplemented!()
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
    vm_space: Arc<VmSpace>,
    /// The parent vmar. If points to none, this is a root vmar
    parent: Weak<Vmar_>,
}

/// FIXME: How can a vmar have its child vmar and vmos with its rights?
struct VmarInner {
    /// Whether the vmar is destroyed
    is_destroyed: bool,
    /// The child vmars. The key is offset relative to root VMAR
    child_vmar_s: BTreeMap<Vaddr, Arc<Vmar_>>,
    /// The mapped vmos. The key is offset relative to root VMAR
    mapped_vmos: BTreeMap<Vaddr, Arc<VmMapping>>,
    /// Free regions that can be used for creating child vmar or mapping vmos
    free_regions: BTreeMap<Vaddr, FreeRegion>,
}

// FIXME: How to set the correct root vmar range?
// We should not include addr 0 here(is this right?), since the 0 addr means the null pointer.
// We should include addr 0x0040_0000, since non-pie executables typically are put on 0x0040_0000.
pub const ROOT_VMAR_LOWEST_ADDR: Vaddr = 0x0010_0000;
pub const ROOT_VMAR_HIGHEST_ADDR: Vaddr = 0x1000_0000_0000;

impl Vmar_ {
    pub fn new_root() -> Result<Self> {
        let mut free_regions = BTreeMap::new();
        let root_region = FreeRegion::new(ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_HIGHEST_ADDR);
        free_regions.insert(root_region.start(), root_region);
        let vmar_inner = VmarInner {
            is_destroyed: false,
            child_vmar_s: BTreeMap::new(),
            mapped_vmos: BTreeMap::new(),
            free_regions,
        };
        let vmar_ = Vmar_ {
            inner: Mutex::new(vmar_inner),
            vm_space: Arc::new(VmSpace::new()),
            base: 0,
            size: ROOT_VMAR_HIGHEST_ADDR,
            parent: Weak::new(),
        };
        Ok(vmar_)
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
        for (vmo_base, vm_mapping) in &self.inner.lock().mapped_vmos {
            let vmo_range = *vmo_base..(*vmo_base + vm_mapping.size());
            if is_intersected(&range, &vmo_range) {
                let intersected_range = get_intersected_range(&range, &vmo_range);
                // TODO: How to protect a mapped vmo?
                todo!()
            }
        }

        for (_, child_vmar_) in &self.inner.lock().child_vmar_s {
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
        for (_, free_region) in &self.inner.lock().free_regions {
            if is_intersected(&free_region.range, &protected_range) {
                return Err(Error::InvalidArgs);
            }
        }

        // if the protected range intersects with child vmar_, child vmar_ is responsible to do the check.
        for (_, child_vmar_) in &self.inner.lock().child_vmar_s {
            let child_range = child_vmar_.range();
            if is_intersected(&child_range, &protected_range) {
                let intersected_range = get_intersected_range(&child_range, &protected_range);
                child_vmar_.check_protected_range(&intersected_range)?;
            }
        }

        Ok(())
    }

    /// Handle user space page fault, if the page fault is successfully handled ,return Ok(()).
    pub fn handle_page_fault(&self, page_fault_addr: Vaddr, write: bool) -> Result<()> {
        if page_fault_addr < self.base || page_fault_addr >= self.base + self.size {
            return Err(Error::AccessDenied);
        }

        let inner = self.inner.lock();
        for (child_vmar_base, child_vmar) in &inner.child_vmar_s {
            if *child_vmar_base <= page_fault_addr
                && page_fault_addr < *child_vmar_base + child_vmar.size
            {
                return child_vmar.handle_page_fault(page_fault_addr, write);
            }
        }

        // FIXME: If multiple vmos are mapped to the addr, should we allow all vmos to handle page fault?
        for (vm_mapping_base, vm_mapping) in &inner.mapped_vmos {
            if *vm_mapping_base <= page_fault_addr
                && page_fault_addr <= *vm_mapping_base + vm_mapping.size()
            {
                return vm_mapping.handle_page_fault(page_fault_addr, write);
            }
        }

        return Err(Error::AccessDenied);
    }

    pub fn destroy_all(&self) -> Result<()> {
        todo!()
    }

    pub fn destroy(&self, range: Range<usize>) -> Result<()> {
        todo!()
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
        for (vm_mapping_base, vm_mapping) in &self.inner.lock().mapped_vmos {
            let vm_mapping_end = *vm_mapping_base + vm_mapping.size();
            if *vm_mapping_base <= read_start && read_end <= vm_mapping_end {
                let vm_mapping_offset = read_start - *vm_mapping_base;
                return vm_mapping.read_bytes(vm_mapping_offset, buf);
            }
        }

        // FIXME: If the read range is across different vmos or child vmars, should we directly return error?
        Err(Error::AccessDenied)
    }

    pub fn write(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let write_start = self.base + offset;
        let write_end = buf.len() + write_start;
        // if the write range is in child vmar
        for (child_vmar_base, child_vmar) in &self.inner.lock().child_vmar_s {
            let child_vmar_end = *child_vmar_base + child_vmar.size;
            if *child_vmar_base <= write_start && write_end <= child_vmar_end {
                let child_offset = write_start - *child_vmar_base;
                return child_vmar.write(child_offset, buf);
            }
        }
        // if the write range is in mapped vmo
        for (vm_mapping_base, vm_mapping) in &self.inner.lock().mapped_vmos {
            let vm_mapping_end = *vm_mapping_base + vm_mapping.size();
            if *vm_mapping_base <= write_start && write_end <= vm_mapping_end {
                let vm_mapping_offset = write_start - *vm_mapping_base;
                return vm_mapping.write_bytes(vm_mapping_offset, buf);
            }
        }

        // FIXME: If the write range is across different vmos or child vmars, should we directly return error?
        Err(Error::AccessDenied)
    }

    /// allocate a child vmar_.
    pub fn alloc_child_vmar(
        self: &Arc<Self>,
        child_vmar_offset: Option<usize>,
        child_vmar_size: usize,
        align: usize,
    ) -> Result<Arc<Vmar_>> {
        match self.find_free_region_for_child(child_vmar_offset, child_vmar_size, align) {
            None => return Err(Error::InvalidArgs),
            Some((region_base, child_vmar_offset)) => {
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
                    mapped_vmos: BTreeMap::new(),
                    free_regions: child_regions,
                };
                let child_vmar_ = Arc::new(Vmar_ {
                    inner: Mutex::new(child_vmar_inner),
                    base: child_vmar_offset,
                    size: child_vmar_size,
                    vm_space: self.vm_space.clone(),
                    parent: Arc::downgrade(self),
                });
                self.inner
                    .lock()
                    .child_vmar_s
                    .insert(child_vmar_.base, child_vmar_.clone());
                Ok(child_vmar_)
            }
        }
    }

    /// find a free region for child vmar or vmo.
    /// returns (region base addr, child real offset)
    fn find_free_region_for_child(
        &self,
        child_offset: Option<Vaddr>,
        child_size: usize,
        align: usize,
    ) -> Option<(Vaddr, Vaddr)> {
        for (region_base, free_region) in &self.inner.lock().free_regions {
            if let Some(child_vmar_offset) = child_offset {
                // if the offset is set, we should find a free region can satisfy both the offset and size
                if *region_base <= child_vmar_offset
                    && (child_vmar_offset + child_size) <= (free_region.end())
                {
                    return Some((*region_base, child_vmar_offset));
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
                    return Some((*region_base, child_vmar_real_start));
                }
            }
        }
        None
    }

    fn range(&self) -> Range<usize> {
        self.base..(self.base + self.size)
    }

    fn check_vmo_overwrite(&self, vmo_range: Range<usize>, can_overwrite: bool) -> Result<()> {
        let inner = self.inner.lock();
        for (_, child_vmar) in &inner.child_vmar_s {
            let child_vmar_range = child_vmar.range();
            if is_intersected(&vmo_range, &child_vmar_range) {
                return Err(Error::InvalidArgs);
            }
        }

        if !can_overwrite {
            for (child_vmo_base, child_vmo) in &inner.mapped_vmos {
                let child_vmo_range = *child_vmo_base..*child_vmo_base + child_vmo.size();
                if is_intersected(&vmo_range, &child_vmo_range) {
                    return Err(Error::InvalidArgs);
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
            .mapped_vmos
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
        let allocate_size = size.max(vmo_size);
        let mut inner = self.inner.lock();
        if can_overwrite {
            // if can_overwrite, the offset is ensured not to be None
            let offset = offset.unwrap();
            let vmo_range = offset..(offset + allocate_size);
            // If can overwrite, the vmo can cross multiple free regions. We will split each free regions that intersect with the vmo
            let mut split_regions = Vec::new();
            for (free_region_base, free_region) in &inner.free_regions {
                let free_region_range = free_region.range();
                if is_intersected(free_region_range, &vmo_range) {
                    split_regions.push(*free_region_base);
                }
            }
            for region_base in split_regions {
                let free_region = inner.free_regions.remove(&region_base).unwrap();
                let intersected_range = get_intersected_range(free_region.range(), &vmo_range);
                let regions_after_split = free_region.allocate_range(intersected_range);
                regions_after_split.into_iter().for_each(|region| {
                    inner.free_regions.insert(region.start(), region);
                });
            }
            return Ok(offset);
        } else {
            // Otherwise, the vmo in a single region
            match self.find_free_region_for_child(offset, allocate_size, align) {
                None => return Err(Error::InvalidArgs),
                Some((free_region_base, offset)) => {
                    let free_region = inner.free_regions.remove(&free_region_base).unwrap();
                    let vmo_range = offset..(offset + allocate_size);
                    let intersected_range = get_intersected_range(free_region.range(), &vmo_range);
                    let regions_after_split = free_region.allocate_range(intersected_range);
                    regions_after_split.into_iter().for_each(|region| {
                        inner.free_regions.insert(region.start(), region);
                    });
                    return Ok(offset);
                }
            }
        }
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
}

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
}

/// determine whether two ranges are intersected.
fn is_intersected(range1: &Range<usize>, range2: &Range<usize>) -> bool {
    range1.start.max(range2.start) < range1.end.min(range2.end)
}

/// get the intersection range of two ranges.
/// The two ranges should be ensured to be intersected.
fn get_intersected_range(range1: &Range<usize>, range2: &Range<usize>) -> Range<usize> {
    debug_assert!(is_intersected(range1, range2));
    range1.start.max(range2.start)..range1.end.min(range2.end)
}
