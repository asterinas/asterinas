//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod options;
mod static_cap;

use crate::rights::Rights;
use alloc::collections::BTreeMap;
use alloc::sync::Arc;
use alloc::sync::Weak;
use alloc::vec::Vec;
use bitflags::bitflags;
use jinux_frame::config::PAGE_SIZE;
// use jinux_frame::vm::VmPerm;
use core::ops::Range;
use jinux_frame::vm::Vaddr;
use jinux_frame::vm::VmIo;
// use jinux_frame::vm::VmPerm;
use jinux_frame::vm::VmSpace;
use jinux_frame::AlignExt;
use jinux_frame::{Error, Result};
use spin::Mutex;

use super::vmo::Vmo;

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

struct Vmar_ {
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
    mapped_vmos: BTreeMap<Vaddr, Arc<Vmo>>,
    /// Free ranges that can be used for creating child vmar or mapping vmos
    free_regions: BTreeMap<Vaddr, FreeRegion>,
}

pub const ROOT_VMAR_HIGHEST_ADDR: Vaddr = 0x1000_0000_0000;

impl Vmar_ {
    pub fn new_root() -> Result<Self> {
        let mut free_regions = BTreeMap::new();
        let root_region = FreeRegion::new(0..ROOT_VMAR_HIGHEST_ADDR);
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
        for (vmo_base, mapped_vmo) in &self.inner.lock().mapped_vmos {
            let vmo_range = *vmo_base..(*vmo_base + mapped_vmo.size());
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
        for (vmo_base, vmo) in &self.inner.lock().mapped_vmos {
            let vmo_end = *vmo_base + vmo.size();
            if *vmo_base <= read_start && read_end <= vmo_end {
                let vmo_offset = read_start - *vmo_base;
                return vmo.read_bytes(vmo_offset, buf);
            }
        }
        // FIXME: should be read the free range?
        // for (_, free_region) in &self.inner.lock().free_regions {
        //     let (region_start, region_end) = free_region.range();
        //     if region_start <= read_start && read_end <= region_end {
        //         return self.vm_space.read_bytes(read_start, buf);
        //     }
        // }

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
        for (vmo_base, vmo) in &self.inner.lock().mapped_vmos {
            let vmo_end = *vmo_base + vmo.size();
            if *vmo_base <= write_start && write_end <= vmo_end {
                let vmo_offset = write_start - *vmo_base;
                return vmo.write_bytes(vmo_offset, buf);
            }
        }
        // if the write range is in free region
        // FIXME: should we write the free region?
        // for (_, free_region) in &self.inner.lock().free_regions {
        //     let (region_start, region_end) = free_region.range();
        //     if region_start <= write_start && write_end <= region_end {
        //         return self.vm_space.write_bytes(write_start, buf);
        //     }
        // }

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
        match self.find_free_region_for_child_vmar(child_vmar_offset, child_vmar_size, align) {
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

    /// returns (region base addr, child real offset)
    fn find_free_region_for_child_vmar(
        &self,
        child_vmar_offset: Option<Vaddr>,
        child_vmar_size: usize,
        align: usize,
    ) -> Option<(Vaddr, Vaddr)> {
        for (region_base, free_region) in &self.inner.lock().free_regions {
            if let Some(child_vmar_offset) = child_vmar_offset {
                // if the offset is set, we should find a free region can satisfy both the offset and size
                if *region_base <= child_vmar_offset
                    && (child_vmar_offset + child_vmar_size) <= (free_region.end())
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
                let child_vmar_real_end = child_vmar_real_start + child_vmar_size;
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

bitflags! {
    /// The memory access permissions of memory mappings.
    pub struct VmPerms: u32 {
        /// Readable.
        const READ    = 1 << 0;
        /// Writable.
        const WRITE   = 1 << 1;
        /// Executable.
        const EXEC   = 1 << 2;
    }
}

impl From<Rights> for VmPerms {
    fn from(rights: Rights) -> VmPerms {
        let mut vm_perm = VmPerms::empty();
        if rights.contains(Rights::READ) {
            vm_perm |= VmPerms::READ;
        }
        if rights.contains(Rights::WRITE) {
            vm_perm |= VmPerms::WRITE;
        }
        if rights.contains(Rights::EXEC) {
            vm_perm |= VmPerms::EXEC;
        }
        vm_perm
    }
}

impl From<VmPerms> for Rights {
    fn from(vm_perms: VmPerms) -> Rights {
        let mut rights = Rights::empty();
        if vm_perms.contains(VmPerms::READ) {
            rights |= Rights::READ;
        }
        if vm_perms.contains(VmPerms::WRITE) {
            rights |= Rights::WRITE;
        }
        if vm_perms.contains(VmPerms::EXEC) {
            rights |= Rights::EXEC;
        }
        rights
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
