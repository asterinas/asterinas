// SPDX-License-Identifier: MPL-2.0

#![allow(dead_code)]
#![allow(unused_variables)]

use core::{
    cmp::{max, min},
    ops::Range,
};

use align_ext::AlignExt;
use aster_rights::Rights;
use ostd::{
    mm::{
        tlb::TlbFlushOp, vm_space::VmItem, CachePolicy, Frame, FrameAllocOptions, PageFlags,
        PageProperty, VmSpace,
    },
    sync::RwLockReadGuard,
};

use super::{interval::Interval, is_intersected, Vmar, Vmar_};
use crate::{
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::{
        perms::VmPerms,
        util::duplicate_frame,
        vmo::{Vmo, VmoRightsOp},
    },
};

/// A `VmMapping` represents mapping a range of physical pages into a `Vmar`.
///
/// A `VmMapping` can bind with a `Vmo` which can provide physical pages for mapping.
/// Otherwise, it must be an anonymous mapping and will map any empty physical page.
/// A `VmMapping` binding with a `Vmo` is called VMO-backed mapping. Generally, a VMO-backed
/// mapping is a file-backed mapping. Yet there are also some situations where specific pages
/// that are not in a file need to be mapped. e.g:
/// - Mappings to the VDSO data.
/// - Shared anonymous mappings. because the mapped pages need to be retained and shared with
///   other processes.
///
/// Such mappings will also be VMO-backed mappings.
pub(super) struct VmMapping {
    inner: RwLock<VmMappingInner>,
    /// The parent VMAR. The parent should always point to a valid VMAR.
    parent: Weak<Vmar_>,
    /// Specific physical pages that need to be mapped.
    /// If this field is `None`, it means that the mapping is
    /// an independent anonymous mapping.
    vmo: Option<MappedVmo>,
    /// Whether the mapping is shared.
    /// The updates to a shared mapping are visible among processes.
    /// or are carried through to the underlying file for
    /// file-backed shared mappings.
    is_shared: bool,
    /// Whether the mapping needs to handle surrounding pages when handling page fault.
    handle_page_faults_around: bool,
}

impl VmMapping {
    pub fn try_clone(&self) -> Result<Self> {
        let inner = self.inner.read().clone();
        let vmo = self.vmo.as_ref().map(|vmo| vmo.dup()).transpose()?;
        Ok(Self {
            inner: RwLock::new(inner),
            parent: self.parent.clone(),
            vmo,
            is_shared: self.is_shared,
            handle_page_faults_around: self.handle_page_faults_around,
        })
    }
}

#[derive(Clone)]
struct VmMappingInner {
    /// For the VMO-backed mapping, this field indicates the map offset of the VMO in bytes.
    vmo_offset: Option<usize>,
    /// The size of mapping, in bytes. The map size can even be larger than the size of VMO.
    /// Those pages outside VMO range cannot be read or write.
    map_size: usize,
    /// The base address relative to the root VMAR where the VMO is mapped.
    map_to_addr: Vaddr,
    /// is destroyed
    is_destroyed: bool,
    /// The permissions of pages in the mapping.
    /// All pages within the same `VmMapping` have the same permissions.
    perms: VmPerms,
}

impl Interval<usize> for Arc<VmMapping> {
    fn range(&self) -> Range<usize> {
        self.map_to_addr()..self.map_to_addr() + self.map_size()
    }
}

impl VmMapping {
    pub fn build_mapping<R1, R2>(option: VmarMapOptions<R1, R2>) -> Result<Self> {
        let VmarMapOptions {
            parent,
            vmo,
            perms,
            vmo_offset,
            vmo_limit,
            size,
            offset,
            align,
            can_overwrite,
            is_shared,
            handle_page_faults_around,
        } = option;
        let Vmar(parent_vmar, _) = parent;
        let map_to_addr =
            parent_vmar.allocate_free_region_for_mapping(size, offset, align, can_overwrite)?;
        trace!(
            "build mapping, map_range = 0x{:x}- 0x{:x}",
            map_to_addr,
            map_to_addr + size
        );

        let (vmo, vmo_offset) = {
            if let Some(vmo) = vmo {
                (
                    Some(MappedVmo::new(vmo.to_dyn(), vmo_offset..vmo_limit)),
                    Some(vmo_offset.align_up(PAGE_SIZE)),
                )
            } else {
                (None, None)
            }
        };

        let vm_mapping_inner = VmMappingInner {
            vmo_offset,
            map_size: size,
            map_to_addr,
            is_destroyed: false,
            perms,
        };

        Ok(Self {
            inner: RwLock::new(vm_mapping_inner),
            parent: Arc::downgrade(&parent_vmar),
            vmo,
            is_shared,
            handle_page_faults_around,
        })
    }

    /// Builds a new VmMapping based on part of current `VmMapping`.
    /// The mapping range of the new mapping must be contained in the full mapping.
    ///
    /// Note: Since such new mappings will intersect with the current mapping,
    /// making sure that when adding the new mapping into a Vmar, the current mapping in the Vmar will be removed.
    fn clone_partial(
        &self,
        range: Range<usize>,
        new_perms: Option<VmPerms>,
    ) -> Result<Arc<VmMapping>> {
        let partial_mapping = Arc::new(self.try_clone()?);
        // Adjust the mapping range and the permission.
        {
            let mut inner = partial_mapping.inner.write();
            inner.shrink_to(range);
            if let Some(perms) = new_perms {
                inner.perms = perms;
            }
        }
        Ok(partial_mapping)
    }

    pub fn vmo(&self) -> Option<&MappedVmo> {
        self.vmo.as_ref()
    }

    /// Returns the mapping's start address.
    pub fn map_to_addr(&self) -> Vaddr {
        self.inner.read().map_to_addr
    }

    /// Returns the mapping's end address.
    pub fn map_end(&self) -> Vaddr {
        let inner = self.inner.read();
        inner.map_to_addr + inner.map_size
    }

    /// Returns the mapping's size.
    pub fn map_size(&self) -> usize {
        self.inner.read().map_size
    }

    /// Unmaps pages in the range
    pub fn unmap(&self, range: &Range<usize>, may_destroy: bool) -> Result<()> {
        let parent = self.parent.upgrade().unwrap();
        let vm_space = parent.vm_space();
        self.inner.write().unmap(vm_space, range, may_destroy)
    }

    pub fn is_destroyed(&self) -> bool {
        self.inner.read().is_destroyed
    }

    /// Returns whether the mapping is a shared mapping.
    pub fn is_shared(&self) -> bool {
        self.is_shared
    }

    pub fn enlarge(&self, extra_size: usize) {
        self.inner.write().map_size += extra_size;
    }

    pub fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        self.check_perms(&page_fault_info.required_perms)?;

        let address = page_fault_info.address;

        let page_aligned_addr = address.align_down(PAGE_SIZE);
        let is_write = page_fault_info.required_perms.contains(VmPerms::WRITE);

        if !is_write && self.vmo.is_some() && self.handle_page_faults_around {
            self.handle_page_faults_around(address)?;
            return Ok(());
        }

        let root_vmar = self.parent.upgrade().unwrap();
        root_vmar.vm_space().cursor_mut_with(
            &(page_aligned_addr..page_aligned_addr + PAGE_SIZE),
            |cursor| {
                match cursor.query().unwrap() {
                    VmItem::Mapped {
                        va,
                        frame,
                        mut prop,
                    } if is_write => {
                        // Perform COW if it is a write access to a shared mapping.

                        // Skip if the page fault is already handled.
                        if prop.flags.contains(PageFlags::W) {
                            return Result::Ok(());
                        }

                        // If the forked child or parent immediately unmaps the page after
                        // the fork without accessing it, we are the only reference to the
                        // frame. We can directly map the frame as writable without
                        // copying. In this case, the reference count of the frame is 2 (
                        // one for the mapping and one for the frame handle itself).
                        let only_reference = frame.reference_count() == 2;

                        let new_flags = PageFlags::W | PageFlags::ACCESSED | PageFlags::DIRTY;

                        if self.is_shared || only_reference {
                            cursor.protect_next(PAGE_SIZE, |p| p.flags |= new_flags);
                            cursor.flusher().issue_tlb_flush(TlbFlushOp::Address(va));
                            cursor.flusher().dispatch_tlb_flush();
                        } else {
                            let new_frame = duplicate_frame(&frame)?;
                            prop.flags |= new_flags;
                            cursor.map(new_frame, prop);
                        }
                    }
                    VmItem::Mapped { .. } => {
                        panic!("non-COW page fault should not happen on mapped address")
                    }
                    VmItem::NotMapped { .. } => {
                        // Map a new frame to the page fault address.

                        let inner = self.inner.read();
                        let (frame, is_readonly) = self.prepare_page(&inner, address, is_write)?;

                        let vm_perms = {
                            let mut perms = inner.perms;
                            if is_readonly {
                                // COW pages are forced to be read-only.
                                perms -= VmPerms::WRITE;
                            }
                            perms
                        };
                        drop(inner);

                        let mut page_flags = vm_perms.into();
                        page_flags |= PageFlags::ACCESSED;
                        if is_write {
                            page_flags |= PageFlags::DIRTY;
                        }
                        let map_prop = PageProperty::new(page_flags, CachePolicy::Writeback);

                        cursor.map(frame, map_prop);
                    }
                }

                Result::Ok(())
            },
        )??;

        Ok(())
    }

    fn prepare_page(
        &self,
        mapping_inner: &RwLockReadGuard<VmMappingInner>,
        page_fault_addr: Vaddr,
        write: bool,
    ) -> Result<(Frame, bool)> {
        let mut is_readonly = false;
        let Some(vmo) = &self.vmo else {
            return Ok((FrameAllocOptions::new(1).alloc_single()?, is_readonly));
        };

        let vmo_offset =
            mapping_inner.vmo_offset.unwrap() + page_fault_addr - mapping_inner.map_to_addr;
        let page_idx = vmo_offset / PAGE_SIZE;
        let Ok(page) = vmo.get_committed_frame(page_idx) else {
            if !self.is_shared {
                // The page index is outside the VMO. This is only allowed in private mapping.
                return Ok((FrameAllocOptions::new(1).alloc_single()?, is_readonly));
            } else {
                return_errno_with_message!(
                    Errno::EFAULT,
                    "could not find a corresponding physical page"
                );
            }
        };

        if !self.is_shared && write {
            // Write access to private VMO-backed mapping. Performs COW directly.
            Ok((duplicate_frame(&page)?, is_readonly))
        } else {
            // Operations to shared mapping or read access to private VMO-backed mapping.
            // If read access to private VMO-backed mapping triggers a page fault,
            // the map should be readonly. If user next tries to write to the frame,
            // another page fault will be triggered which will performs a COW (Copy-On-Write).
            is_readonly = !self.is_shared;
            Ok((page, is_readonly))
        }
    }

    fn handle_page_faults_around(&self, page_fault_addr: Vaddr) -> Result<()> {
        const SURROUNDING_PAGE_NUM: usize = 16;
        const SURROUNDING_PAGE_ADDR_MASK: usize = !(SURROUNDING_PAGE_NUM * PAGE_SIZE - 1);

        let inner = self.inner.read();
        let vmo_offset = inner.vmo_offset.unwrap();
        let vmo = self.vmo().unwrap();
        let around_page_addr = page_fault_addr & SURROUNDING_PAGE_ADDR_MASK;
        let valid_size = min(vmo.size().saturating_sub(vmo_offset), inner.map_size);

        let start_addr = max(around_page_addr, inner.map_to_addr);
        let end_addr = min(
            start_addr + SURROUNDING_PAGE_NUM * PAGE_SIZE,
            inner.map_to_addr + valid_size,
        );

        let vm_perms = inner.perms - VmPerms::WRITE;
        let parent = self.parent.upgrade().unwrap();
        let vm_space = parent.vm_space();
        vm_space.cursor_mut_with(&(start_addr..end_addr), |cursor| {
            let operate = move |commit_fn: &mut dyn FnMut() -> Result<Frame>| {
                if let VmItem::NotMapped { va, len } = cursor.query().unwrap() {
                    // We regard all the surrounding pages as accessed, no matter
                    // if it is really so. Then the hardware won't bother to update
                    // the accessed bit of the page table on following accesses.
                    let page_flags = PageFlags::from(vm_perms) | PageFlags::ACCESSED;
                    let page_prop = PageProperty::new(page_flags, CachePolicy::Writeback);
                    let frame = commit_fn()?;
                    cursor.map(frame, page_prop);
                } else {
                    let next_addr = cursor.virt_addr() + PAGE_SIZE;
                    if next_addr < end_addr {
                        let _ = cursor.jump(next_addr);
                    }
                }
                Ok(())
            };

            let start_offset = vmo_offset + start_addr - inner.map_to_addr;
            let end_offset = vmo_offset + end_addr - inner.map_to_addr;
            vmo.operate_on_range(&(start_offset..end_offset), operate)?;

            Result::Ok(())
        })??;

        Ok(())
    }

    /// Protects a specified range of pages in the mapping to the target perms.
    /// This `VmMapping` will split to maintain its property.
    ///
    /// Since this method will modify the `vm_mappings` in the vmar,
    /// it should not be called during the direct iteration of the `vm_mappings`.
    pub(super) fn protect(&self, new_perms: VmPerms, range: Range<usize>) -> Result<()> {
        // If `new_perms` is equal to `old_perms`, `protect()` will not modify any permission in the VmMapping.
        let old_perms = self.inner.read().perms;
        if old_perms == new_perms {
            return Ok(());
        }

        // Protect permission for the perm in the VmMapping.
        self.protect_with_subdivision(&range, new_perms)?;
        // Protect permission in the VmSpace.
        let vmar = self.parent.upgrade().unwrap();
        let vm_space = vmar.vm_space();
        self.inner.write().protect(vm_space, new_perms, range)?;

        Ok(())
    }

    pub(super) fn new_fork(&self, new_parent: &Arc<Vmar_>) -> Result<VmMapping> {
        let new_inner = self.inner.read().clone();

        Ok(VmMapping {
            inner: RwLock::new(new_inner),
            parent: Arc::downgrade(new_parent),
            vmo: self.vmo.as_ref().map(|vmo| vmo.dup()).transpose()?,
            is_shared: self.is_shared,
            handle_page_faults_around: self.handle_page_faults_around,
        })
    }

    pub fn range(&self) -> Range<usize> {
        self.map_to_addr()..self.map_to_addr() + self.map_size()
    }

    /// Protects the current `VmMapping` to enforce new permissions within a specified range.
    ///
    /// Due to the property of `VmMapping`, this operation may require subdividing the current
    /// `VmMapping`. In this condition, it will generate a new `VmMapping` with the specified `perm` to protect the
    /// target range, as well as additional `VmMappings` to preserve the mappings in the remaining ranges.
    ///
    /// There are four conditions:
    /// 1. |--------old perm--------| -> |-old-| + |------new------|
    /// 2. |--------old perm--------| -> |-new-| + |------old------|
    /// 3. |--------old perm--------| -> |-old-| + |-new-| + |-old-|
    /// 4. |--------old perm--------| -> |---------new perm--------|
    ///
    /// Generally, this function is only used in `protect()` method.
    /// This method modifies the parent `Vmar` in the end if subdividing is required.
    /// It removes current mapping and add split mapping to the Vmar.
    fn protect_with_subdivision(
        &self,
        intersect_range: &Range<usize>,
        perms: VmPerms,
    ) -> Result<()> {
        let mut additional_mappings = Vec::new();
        let range = self.range();
        // Condition 4, the `additional_mappings` will be empty.
        if range.start == intersect_range.start && range.end == intersect_range.end {
            self.inner.write().perms = perms;
            return Ok(());
        }
        // Condition 1 or 3, which needs an additional new VmMapping with range (range.start..intersect_range.start)
        if range.start < intersect_range.start {
            let additional_left_mapping =
                self.clone_partial(range.start..intersect_range.start, None)?;
            additional_mappings.push(additional_left_mapping);
        }
        // Condition 2 or 3, which needs an additional new VmMapping with range (intersect_range.end..range.end).
        if range.end > intersect_range.end {
            let additional_right_mapping =
                self.clone_partial(intersect_range.end..range.end, None)?;
            additional_mappings.push(additional_right_mapping);
        }
        // The protected VmMapping must exist and its range is `intersect_range`.
        let protected_mapping = self.clone_partial(intersect_range.clone(), Some(perms))?;

        // Begin to modify the `Vmar`.
        let vmar = self.parent.upgrade().unwrap();
        let mut vmar_inner = vmar.inner.write();
        // Remove the original mapping.
        vmar_inner.vm_mappings.remove(&self.map_to_addr());
        // Add protected mappings to the vmar.
        vmar_inner
            .vm_mappings
            .insert(protected_mapping.map_to_addr(), protected_mapping);
        // Add additional mappings to the vmar.
        for mapping in additional_mappings {
            vmar_inner
                .vm_mappings
                .insert(mapping.map_to_addr(), mapping);
        }

        Ok(())
    }

    /// Trims a range from the mapping.
    /// There are several cases.
    ///  1. the trim_range is totally in the mapping. Then the mapping will split as two mappings.
    ///  2. the trim_range covers the mapping. Then the mapping will be destroyed.
    ///  3. the trim_range partly overlaps with the mapping, in left or right. Only overlapped part is trimmed.
    ///     If we create a mapping with a new map addr, we will add it to mappings_to_append.
    ///     If the mapping with map addr does not exist ever, the map addr will be added to mappings_to_remove.
    ///     Otherwise, we will directly modify self.
    pub fn trim_mapping(
        self: &Arc<Self>,
        trim_range: &Range<usize>,
        mappings_to_remove: &mut LinkedList<Vaddr>,
        mappings_to_append: &mut LinkedList<(Vaddr, Arc<VmMapping>)>,
    ) -> Result<()> {
        let map_to_addr = self.map_to_addr();
        let map_size = self.map_size();
        let range = self.range();
        if !is_intersected(&range, trim_range) {
            return Ok(());
        }
        if trim_range.start <= map_to_addr && trim_range.end >= map_to_addr + map_size {
            // Fast path: the whole mapping was trimmed.
            self.unmap(trim_range, true)?;
            mappings_to_remove.push_back(map_to_addr);
            return Ok(());
        }
        if trim_range.start <= range.start {
            mappings_to_remove.push_back(map_to_addr);
            if trim_range.end <= range.end {
                // Overlap vm_mapping from left.
                let new_map_addr = self.trim_left(trim_range.end)?;
                mappings_to_append.push_back((new_map_addr, self.clone()));
            } else {
                // The mapping was totally destroyed.
            }
        } else {
            if trim_range.end <= range.end {
                // The trim range was totally inside the old mapping.
                let another_mapping = Arc::new(self.try_clone()?);
                let another_map_to_addr = another_mapping.trim_left(trim_range.end)?;
                mappings_to_append.push_back((another_map_to_addr, another_mapping));
            } else {
                // Overlap vm_mapping from right.
            }
            self.trim_right(trim_range.start)?;
        }

        Ok(())
    }

    /// Trims the mapping from left to a new address.
    fn trim_left(&self, vaddr: Vaddr) -> Result<Vaddr> {
        let vmar = self.parent.upgrade().unwrap();
        let vm_space = vmar.vm_space();
        self.inner.write().trim_left(vm_space, vaddr)
    }

    /// Trims the mapping from right to a new address.
    fn trim_right(&self, vaddr: Vaddr) -> Result<Vaddr> {
        let vmar = self.parent.upgrade().unwrap();
        let vm_space = vmar.vm_space();
        self.inner.write().trim_right(vm_space, vaddr)
    }

    fn check_perms(&self, perms: &VmPerms) -> Result<()> {
        self.inner.read().check_perms(perms)
    }
}

impl VmMappingInner {
    /// Unmap pages in the range.
    fn unmap(&mut self, vm_space: &VmSpace, range: &Range<usize>, may_destroy: bool) -> Result<()> {
        let map_addr = range.start.align_down(PAGE_SIZE);
        let map_end = range.end.align_up(PAGE_SIZE);
        let map_range = map_addr..map_end;
        vm_space.cursor_mut_with(&map_range, |cursor| {
            cursor.unmap(map_range.len());
        })?;

        if may_destroy && map_range == self.range() {
            self.is_destroyed = true;
        }
        Ok(())
    }

    pub(super) fn protect(
        &mut self,
        vm_space: &VmSpace,
        perms: VmPerms,
        range: Range<usize>,
    ) -> Result<()> {
        debug_assert!(range.start % PAGE_SIZE == 0);
        debug_assert!(range.end % PAGE_SIZE == 0);
        vm_space
            .cursor_mut_with(&range, |cursor| {
                let op = |p: &mut PageProperty| p.flags = perms.into();
                while cursor.virt_addr() < range.end {
                    if let Some(va) = cursor.protect_next(range.end - cursor.virt_addr(), op) {
                        cursor.flusher().issue_tlb_flush(TlbFlushOp::Range(va));
                    } else {
                        break;
                    }
                }
                cursor.flusher().dispatch_tlb_flush();
            })
            .unwrap();
        Ok(())
    }

    /// Trim the mapping from left to a new address.
    fn trim_left(&mut self, vm_space: &VmSpace, vaddr: Vaddr) -> Result<Vaddr> {
        trace!(
            "trim left: range: {:x?}, vaddr = 0x{:x}",
            self.range(),
            vaddr
        );
        debug_assert!(vaddr >= self.map_to_addr && vaddr <= self.map_to_addr + self.map_size);
        debug_assert!(vaddr % PAGE_SIZE == 0);
        let trim_size = vaddr - self.map_to_addr;

        self.unmap(vm_space, &(self.map_to_addr..vaddr), true)?;

        self.map_to_addr = vaddr;
        self.vmo_offset = self.vmo_offset.map(|vmo_offset| vmo_offset + trim_size);
        self.map_size -= trim_size;

        Ok(self.map_to_addr)
    }

    /// Trim the mapping from right to a new address.
    fn trim_right(&mut self, vm_space: &VmSpace, vaddr: Vaddr) -> Result<Vaddr> {
        trace!(
            "trim right: range: {:x?}, vaddr = 0x{:x}",
            self.range(),
            vaddr
        );
        debug_assert!(vaddr >= self.map_to_addr && vaddr <= self.map_to_addr + self.map_size);
        debug_assert!(vaddr % PAGE_SIZE == 0);

        self.unmap(vm_space, &(vaddr..self.map_to_addr + self.map_size), true)?;

        self.map_size = vaddr - self.map_to_addr;
        Ok(self.map_to_addr)
    }

    /// Shrinks the current `VmMapping` to the new range.
    /// The new range must be contained in the old range.
    fn shrink_to(&mut self, new_range: Range<usize>) {
        debug_assert!(self.map_to_addr <= new_range.start);
        debug_assert!(self.map_to_addr + self.map_size >= new_range.end);
        self.vmo_offset = self
            .vmo_offset
            .map(|vmo_offset| vmo_offset + new_range.start - self.map_to_addr);
        self.map_to_addr = new_range.start;
        self.map_size = new_range.end - new_range.start;
    }

    fn range(&self) -> Range<usize> {
        self.map_to_addr..self.map_to_addr + self.map_size
    }

    fn check_perms(&self, perms: &VmPerms) -> Result<()> {
        if !self.perms.contains(*perms) {
            return_errno_with_message!(Errno::EACCES, "perm check fails");
        }
        Ok(())
    }
}

/// Options for creating a new mapping. The mapping is not allowed to overlap
/// with any child VMARs. And unless specified otherwise, it is not allowed
/// to overlap with any existing mapping, either.
pub struct VmarMapOptions<R1, R2> {
    parent: Vmar<R1>,
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

impl<R1, R2> VmarMapOptions<R1, R2> {
    /// Creates a default set of options with the VMO and the memory access
    /// permissions.
    ///
    /// The VMO must have access rights that correspond to the memory
    /// access permissions. For example, if `perms` contains `VmPerms::Write`,
    /// then `vmo.rights()` should contain `Rights::WRITE`.
    pub fn new(parent: Vmar<R1>, size: usize, perms: VmPerms) -> Self {
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

    /// Creates the mapping.
    ///
    /// All options will be checked at this point.
    ///
    /// On success, the virtual address of the new mapping is returned.
    pub fn build(self) -> Result<Vaddr> {
        self.check_options()?;
        let parent_vmar = self.parent.0.clone();
        let vm_mapping = Arc::new(VmMapping::build_mapping(self)?);
        let map_to_addr = vm_mapping.map_to_addr();
        parent_vmar.add_mapping(vm_mapping);
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
        self.check_overwrite()?;
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

    /// Checks whether the mapping will overwrite with any existing mapping or vmar.
    fn check_overwrite(&self) -> Result<()> {
        if self.can_overwrite {
            // If `can_overwrite` is set, the offset cannot be None.
            debug_assert!(self.offset.is_some());
            if self.offset.is_none() {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "offset can not be none when can overwrite is true"
                );
            }
        }
        if self.offset.is_none() {
            // If does not specify the offset, we assume the map can always find suitable free region.
            // FIXME: is this always true?
            return Ok(());
        }
        let offset = self.offset.unwrap();
        // We should spare enough space at least for the whole mapping.
        let size = self.size;
        let mapping_range = offset..(offset + size);
        self.parent
            .0
            .check_overwrite(mapping_range, self.can_overwrite)
    }
}

/// A wrapper that represents a mapped [`Vmo`] and provide required functionalities
/// that need to be provided to mappings from the VMO.
pub(super) struct MappedVmo {
    vmo: Vmo,
    /// Represents the accessible range in the VMO for mappings.
    range: Range<usize>,
}

impl MappedVmo {
    /// Creates a `MappedVmo` used for mapping.
    fn new(vmo: Vmo, range: Range<usize>) -> Self {
        Self { vmo, range }
    }

    /// Gets the committed frame at the input `page_idx` in the mapped VMO.
    ///
    /// If the VMO has not committed a frame at this index, it will commit
    /// one first and return it.
    pub fn get_committed_frame(&self, page_idx: usize) -> Result<Frame> {
        debug_assert!(self.range.contains(&(page_idx * PAGE_SIZE)));

        self.vmo.commit_page(page_idx * PAGE_SIZE)
    }

    /// Traverses the indices within a specified range of a VMO sequentially.
    /// For each index position, you have the option to commit the page as well as
    /// perform other operations.
    fn operate_on_range<F>(&self, range: &Range<usize>, operate: F) -> Result<()>
    where
        F: FnMut(&mut dyn FnMut() -> Result<Frame>) -> Result<()>,
    {
        debug_assert!(self.range.start <= range.start && self.range.end >= range.end);

        self.vmo.operate_on_range(range, operate)
    }

    /// Duplicates the capability.
    pub fn dup(&self) -> Result<Self> {
        Ok(Self {
            vmo: self.vmo.dup()?,
            range: self.range.clone(),
        })
    }

    /// Returns the size (in bytes) of a VMO.
    pub fn size(&self) -> usize {
        self.vmo.size()
    }
}
