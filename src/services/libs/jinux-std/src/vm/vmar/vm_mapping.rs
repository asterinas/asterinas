use crate::prelude::*;
use core::ops::Range;
use jinux_frame::vm::VmMapOptions;
use jinux_frame::vm::{VmFrameVec, VmIo, VmPerm};
use spin::Mutex;

use crate::vm::{
    vmo::get_page_idx_range,
    vmo::{Vmo, VmoChildOptions},
};

use super::{is_intersected, Vmar, Vmar_};
use crate::vm::perms::VmPerms;
use crate::vm::vmar::Rights;
use crate::vm::vmo::VmoRightsOp;

/// A VmMapping represents mapping a vmo into a vmar.
/// A vmar can has multiple VmMappings, which means multiple vmos are mapped to a vmar.
/// A vmo can also contain multiple VmMappings, which means a vmo can be mapped to multiple vmars.
/// The reltionship between Vmar and Vmo is M:N.
pub struct VmMapping {
    inner: Mutex<VmMappingInner>,
    /// The parent vmar. The parent should always point to a valid vmar.
    parent: Weak<Vmar_>,
    /// The mapped vmo. The mapped vmo is with dynamic capability.
    vmo: Vmo<Rights>,
}

impl VmMapping {
    pub fn try_clone(&self) -> Result<Self> {
        let inner = self.inner.lock().clone();
        let vmo = self.vmo.dup()?;
        Ok(Self {
            inner: Mutex::new(inner),
            parent: self.parent.clone(),
            vmo,
        })
    }
}

#[derive(Clone)]
struct VmMappingInner {
    /// The map offset of the vmo, in bytes.
    vmo_offset: usize,
    /// The size of mapping, in bytes. The map size can even be larger than the size of backup vmo.
    /// Those pages outside vmo range cannot be read or write.
    map_size: usize,
    /// The base address relative to the root vmar where the vmo is mapped.
    map_to_addr: Vaddr,
    /// is destroyed
    is_destroyed: bool,
    /// The pages already mapped. The key is the page index in vmo.
    mapped_pages: BTreeSet<usize>,
    /// The permission of each page. The key is the page index in vmo.
    /// This map can be filled when mapping a vmo to vmar and can be modified when call mprotect.
    /// We keep the options in case the page is not committed(or create copy on write mappings) and will further need these options.
    page_perms: BTreeMap<usize, VmPerm>,
}

impl VmMapping {
    pub fn build_mapping<R1, R2>(option: VmarMapOptions<R1, R2>) -> Result<Self> {
        let VmarMapOptions {
            parent,
            vmo,
            perms,
            vmo_offset,
            size,
            offset,
            align,
            can_overwrite,
        } = option;
        let Vmar(parent_vmar, _) = parent;
        let vmo = vmo.to_dyn();
        let vmo_size = vmo.size();
        let map_to_addr = parent_vmar.allocate_free_region_for_vmo(
            vmo_size,
            size,
            offset,
            align,
            can_overwrite,
        )?;
        debug!(
            "build mapping, map_to_addr = 0x{:x}- 0x{:x}",
            map_to_addr,
            map_to_addr + size
        );
        let mut page_perms = BTreeMap::new();
        let real_map_size = size.min(vmo_size);

        let perm = VmPerm::from(perms);
        let page_idx_range = get_page_idx_range(&(vmo_offset..vmo_offset + size));
        for page_idx in page_idx_range {
            page_perms.insert(page_idx, perm);
        }

        let vm_space = parent_vmar.vm_space();
        let mut mapped_pages = BTreeSet::new();
        let mapped_page_idx_range = get_page_idx_range(&(vmo_offset..vmo_offset + real_map_size));
        let start_page_idx = mapped_page_idx_range.start;
        for page_idx in mapped_page_idx_range {
            let mut vm_map_options = VmMapOptions::new();
            let page_map_addr = map_to_addr + (page_idx - start_page_idx) * PAGE_SIZE;
            vm_map_options.addr(Some(page_map_addr));
            vm_map_options.perm(perm.clone());
            vm_map_options.can_overwrite(can_overwrite);
            vm_map_options.align(align);
            if let Ok(frames) = vmo.get_backup_frame(page_idx, false, false) {
                vm_space.map(frames, &vm_map_options)?;
                mapped_pages.insert(page_idx);
            }
        }

        let vm_mapping_inner = VmMappingInner {
            vmo_offset,
            map_size: size,
            map_to_addr,
            is_destroyed: false,
            mapped_pages,
            page_perms,
        };
        Ok(Self {
            inner: Mutex::new(vm_mapping_inner),
            parent: Arc::downgrade(&parent_vmar),
            vmo,
        })
    }

    pub fn vmo(&self) -> &Vmo<Rights> {
        &self.vmo
    }

    /// Add a new committed page and map it to vmspace. If copy on write is set, it's allowed to unmap the page at the same address.
    /// FIXME: This implementation based on the truth that we map one page at a time. If multiple pages are mapped together, this implementation may have problems
    pub(super) fn map_one_page(&self, page_idx: usize, frames: VmFrameVec) -> Result<()> {
        let parent = self.parent.upgrade().unwrap();
        let vm_space = parent.vm_space();
        let map_addr = page_idx * PAGE_SIZE + self.map_to_addr();
        let vm_perm = self.inner.lock().page_perms.get(&page_idx).unwrap().clone();
        let mut vm_map_options = VmMapOptions::new();
        vm_map_options.addr(Some(map_addr));
        vm_map_options.perm(vm_perm.clone());
        // copy on write allows unmap the mapped page
        if self.vmo.is_cow_child() && vm_space.is_mapped(map_addr) {
            vm_space.unmap(&(map_addr..(map_addr + PAGE_SIZE))).unwrap();
        }
        vm_space.map(frames, &vm_map_options)?;
        self.inner.lock().mapped_pages.insert(page_idx);
        Ok(())
    }

    /// unmap a page
    pub(super) fn unmap_one_page(&self, page_idx: usize) -> Result<()> {
        let parent = self.parent.upgrade().unwrap();
        let vm_space = parent.vm_space();
        let map_addr = page_idx * PAGE_SIZE + self.map_to_addr();
        let range = map_addr..(map_addr + PAGE_SIZE);
        if vm_space.is_mapped(map_addr) {
            vm_space.unmap(&range)?;
        }
        self.inner.lock().mapped_pages.remove(&page_idx);
        Ok(())
    }

    /// the mapping's start address
    pub fn map_to_addr(&self) -> Vaddr {
        self.inner.lock().map_to_addr
    }

    /// the mapping's size
    pub fn map_size(&self) -> usize {
        self.inner.lock().map_size
    }

    /// the vmo_offset
    pub fn vmo_offset(&self) -> usize {
        self.inner.lock().vmo_offset
    }
    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let vmo_read_offset = self.vmo_offset() + offset;
        self.vmo.read_bytes(vmo_read_offset, buf)?;
        Ok(())
    }

    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let vmo_write_offset = self.vmo_offset() + offset;
        self.vmo.write_bytes(vmo_write_offset, buf)?;
        Ok(())
    }

    /// Unmap pages in the range
    pub fn unmap(&self, range: Range<usize>, destroy: bool) -> Result<()> {
        let map_to_addr = self.map_to_addr();
        let vmo_map_range = (range.start - map_to_addr)..(range.end - map_to_addr);
        let page_idx_range = get_page_idx_range(&vmo_map_range);
        for page_idx in page_idx_range {
            self.unmap_one_page(page_idx)?;
        }
        if destroy && range == self.range() {
            self.inner.lock().is_destroyed = false;
        }
        Ok(())
    }

    pub fn unmap_and_decommit(&self, range: Range<usize>) -> Result<()> {
        let map_to_addr = self.map_to_addr();
        let vmo_range = (range.start - map_to_addr)..(range.end - map_to_addr);
        self.unmap(range, false)?;
        self.vmo.decommit(vmo_range)?;
        Ok(())
    }

    pub fn is_destroyed(&self) -> bool {
        self.inner.lock().is_destroyed
    }

    pub fn handle_page_fault(
        &self,
        page_fault_addr: Vaddr,
        not_present: bool,
        write: bool,
    ) -> Result<()> {
        let vmo_offset = self.vmo_offset() + page_fault_addr - self.map_to_addr();
        if vmo_offset >= self.vmo.size() {
            return_errno_with_message!(Errno::EACCES, "page fault addr is not backed up by a vmo");
        }
        let page_idx = vmo_offset / PAGE_SIZE;
        if write {
            self.vmo.check_rights(Rights::WRITE)?;
        } else {
            self.vmo.check_rights(Rights::READ)?;
        }
        self.check_perm(&page_idx, write)?;

        // get the backup frame for page
        let frames = self.vmo.get_backup_frame(page_idx, write, true)?;
        // map the page
        self.map_one_page(page_idx, frames)
    }

    pub(super) fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        let rights = Rights::from(perms);
        self.vmo().check_rights(rights)?;
        debug_assert!(range.start % PAGE_SIZE == 0);
        debug_assert!(range.end % PAGE_SIZE == 0);
        let map_to_addr = self.map_to_addr();
        let start_page = (range.start - map_to_addr) / PAGE_SIZE;
        let end_page = (range.end - map_to_addr) / PAGE_SIZE;
        let vmar = self.parent.upgrade().unwrap();
        let vm_space = vmar.vm_space();
        let perm = VmPerm::from(perms);
        let mut inner = self.inner.lock();
        for page_idx in start_page..end_page {
            inner.page_perms.insert(page_idx, perm);
            let page_addr = page_idx * PAGE_SIZE + map_to_addr;
            if vm_space.is_mapped(page_addr) {
                // if the page is already mapped, we will modify page table
                let perm = VmPerm::from(perms);
                let page_range = page_addr..(page_addr + PAGE_SIZE);
                vm_space.protect(&page_range, perm)?;
            }
        }

        Ok(())
    }

    pub(super) fn fork_mapping(&self, new_parent: Weak<Vmar_>) -> Result<VmMapping> {
        let VmMapping { inner, parent, vmo } = self;
        let parent_vmo = vmo.dup().unwrap();
        let vmo_size = parent_vmo.size();
        let child_vmo = VmoChildOptions::new_cow(parent_vmo, 0..vmo_size).alloc()?;
        let parent_vmar = new_parent.upgrade().unwrap();
        let vm_space = parent_vmar.vm_space();

        let vmo_offset = self.vmo_offset();
        let map_to_addr = self.map_to_addr();
        let map_size = self.map_size();

        let real_map_size = self.map_size().min(child_vmo.size());
        let page_idx_range = get_page_idx_range(&(vmo_offset..vmo_offset + real_map_size));
        let start_page_idx = page_idx_range.start;
        let mut mapped_pages = BTreeSet::new();

        for page_idx in page_idx_range {
            // When map pages from parent, we should forbid write access to these pages.
            // So any write access to these pages will trigger a page fault. Then, we can allocate new pages for the page.
            let mut vm_perm = inner.lock().page_perms.get(&page_idx).unwrap().clone();
            vm_perm -= VmPerm::W;
            let mut vm_map_options = VmMapOptions::new();
            let map_addr = (page_idx - start_page_idx) * PAGE_SIZE + map_to_addr;
            vm_map_options.addr(Some(map_addr));
            vm_map_options.perm(vm_perm);
            if let Ok(frames) = child_vmo.get_backup_frame(page_idx, false, false) {
                vm_space.map(frames, &vm_map_options)?;
                mapped_pages.insert(page_idx);
            }
        }
        let is_destroyed = inner.lock().is_destroyed;
        let page_perms = inner.lock().page_perms.clone();
        let inner = VmMappingInner {
            is_destroyed,
            mapped_pages,
            page_perms,
            vmo_offset,
            map_size,
            map_to_addr,
        };
        Ok(VmMapping {
            inner: Mutex::new(inner),
            parent: new_parent,
            vmo: child_vmo,
        })
    }

    pub fn range(&self) -> Range<usize> {
        self.map_to_addr()..self.map_to_addr() + self.map_size()
    }

    /// Trim a range from the mapping.
    /// There are several cases.
    /// 1. the trim_range is totally in the mapping. Then the mapping will split as two mappings.
    /// 2. the trim_range covers the mapping. Then the mapping will be destroyed.
    /// 3. the trim_range partly overlaps with the mapping, in left or right. Only overlapped part is trimmed.
    /// If we create a mapping with a new map addr, we will add it to mappings_to_append.
    /// If the mapping with map addr does not exist ever, the map addr will be added to mappings_to_remove.
    /// Otherwise, we will directly modify self.
    pub fn trim_mapping(
        self: &Arc<Self>,
        trim_range: &Range<usize>,
        mappings_to_remove: &mut BTreeSet<Vaddr>,
        mappings_to_append: &mut BTreeMap<Vaddr, Arc<VmMapping>>,
    ) -> Result<()> {
        let map_to_addr = self.map_to_addr();
        let map_size = self.map_size();
        let range = self.range();
        if !is_intersected(&range, &trim_range) {
            return Ok(());
        }
        let vm_mapping_left = range.start;
        let vm_mapping_right = range.end;

        if trim_range.start <= vm_mapping_left {
            mappings_to_remove.insert(map_to_addr);
            if trim_range.end <= vm_mapping_right {
                // overlap vm_mapping from left
                let new_map_addr = self.trim_left(trim_range.end)?;
                mappings_to_append.insert(new_map_addr, self.clone());
            } else {
                // the mapping was totally destroyed
            }
        } else {
            if trim_range.end <= vm_mapping_right {
                // the trim range was totally inside the old mapping
                let another_mapping = Arc::new(self.try_clone()?);
                let another_map_to_addr = another_mapping.trim_left(trim_range.end)?;
                mappings_to_append.insert(another_map_to_addr, another_mapping);
            } else {
                // overlap vm_mapping from right
            }
            self.trim_right(trim_range.start)?;
        }

        Ok(())
    }

    /// trim the mapping from left to a new address.
    fn trim_left(&self, vaddr: Vaddr) -> Result<Vaddr> {
        trace!(
            "trim left: range: {:x?}, vaddr = 0x{:x}",
            self.range(),
            vaddr
        );
        let old_left_addr = self.map_to_addr();
        let old_right_addr = self.map_to_addr() + self.map_size();
        debug_assert!(vaddr >= old_left_addr && vaddr <= old_right_addr);
        debug_assert!(vaddr % PAGE_SIZE == 0);
        let trim_size = vaddr - old_left_addr;

        let mut inner = self.inner.lock();
        inner.map_to_addr = vaddr;
        inner.vmo_offset = inner.vmo_offset + trim_size;
        inner.map_size = inner.map_size - trim_size;
        let mut pages_to_unmap = Vec::new();
        for page_idx in 0..trim_size / PAGE_SIZE {
            inner.page_perms.remove(&page_idx);
            if inner.mapped_pages.remove(&page_idx) {
                pages_to_unmap.push(page_idx);
            }
        }

        // release lock here to avoid dead lock
        drop(inner);
        for page_idx in pages_to_unmap {
            let _ = self.unmap_one_page(page_idx);
        }
        Ok(self.map_to_addr())
    }

    /// trim the mapping from right to a new address.
    fn trim_right(&self, vaddr: Vaddr) -> Result<Vaddr> {
        trace!(
            "trim right: range: {:x?}, vaddr = 0x{:x}",
            self.range(),
            vaddr
        );
        let map_to_addr = self.map_to_addr();
        let old_left_addr = map_to_addr;
        let old_right_addr = map_to_addr + self.map_size();
        debug_assert!(vaddr >= old_left_addr && vaddr <= old_right_addr);
        debug_assert!(vaddr % PAGE_SIZE == 0);
        let trim_size = old_right_addr - vaddr;
        let trim_page_idx =
            (vaddr - map_to_addr) / PAGE_SIZE..(old_right_addr - map_to_addr) / PAGE_SIZE;
        for page_idx in trim_page_idx.clone() {
            self.inner.lock().page_perms.remove(&page_idx);
            let _ = self.unmap_one_page(page_idx);
        }
        self.inner.lock().map_size = vaddr - map_to_addr;
        Ok(map_to_addr)
    }

    fn check_perm(&self, page_idx: &usize, write: bool) -> Result<()> {
        let inner = self.inner.lock();
        let page_perm = inner
            .page_perms
            .get(&page_idx)
            .ok_or(Error::with_message(Errno::EINVAL, "invalid page idx"))?;
        if !page_perm.contains(VmPerm::R) {
            return_errno_with_message!(Errno::EINVAL, "perm should at least contain read");
        }
        if write && !page_perm.contains(VmPerm::W) {
            return_errno_with_message!(Errno::EINVAL, "perm should contain write for write access");
        }

        Ok(())
    }
}

/// Options for creating a new mapping. The mapping is not allowed to overlap
/// with any child VMARs. And unless specified otherwise, it is not allowed
/// to overlap with any existing mapping, either.
pub struct VmarMapOptions<R1, R2> {
    parent: Vmar<R1>,
    vmo: Vmo<R2>,
    perms: VmPerms,
    vmo_offset: usize,
    size: usize,
    offset: Option<usize>,
    align: usize,
    can_overwrite: bool,
}

impl<R1, R2> VmarMapOptions<R1, R2> {
    /// Creates a default set of options with the VMO and the memory access
    /// permissions.
    ///
    /// The VMO must have access rights that correspond to the memory
    /// access permissions. For example, if `perms` contains `VmPerm::Write`,
    /// then `vmo.rights()` should contain `Rights::WRITE`.
    pub fn new(parent: Vmar<R1>, vmo: Vmo<R2>, perms: VmPerms) -> Self {
        let size = vmo.size();
        Self {
            parent,
            vmo,
            perms,
            vmo_offset: 0,
            size,
            offset: None,
            align: PAGE_SIZE,
            can_overwrite: false,
        }
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

    /// Sets the size of the mapping.
    ///
    /// The size of a mapping may not be equal to that of the VMO.
    /// For example, it is ok to create a mapping whose size is larger than
    /// that of the VMO, although one cannot read from or write to the
    /// part of the mapping that is not backed by the VMO.
    /// So you may wonder: what is the point of supporting such _oversized_
    /// mappings?  The reason is two-fold.
    /// 1. VMOs are resizable. So even if a mapping is backed by a VMO whose
    /// size is equal to that of the mapping initially, we cannot prevent
    /// the VMO from shrinking.
    /// 2. Mappings are not allowed to overlap by default. As a result,
    /// oversized mappings can serve as a placeholder to prevent future
    /// mappings from occupying some particular address ranges accidentally.
    ///
    /// The default value is the size of the VMO.
    pub fn size(mut self, size: usize) -> Self {
        self.size = size;
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

    /// Creates the mapping.
    ///
    /// All options will be checked at this point.
    ///
    /// On success, the virtual address of the new mapping is returned.
    pub fn build(self) -> Result<Vaddr> {
        self.check_options()?;
        let parent_vmar = self.parent.0.clone();
        let vmo_ = self.vmo.0.clone();
        let vm_mapping = Arc::new(VmMapping::build_mapping(self)?);
        let map_to_addr = vm_mapping.map_to_addr();
        parent_vmar.add_mapping(vm_mapping);
        Ok(map_to_addr)
    }

    /// check whether all options are valid
    fn check_options(&self) -> Result<()> {
        // check align
        debug_assert!(self.align % PAGE_SIZE == 0);
        debug_assert!(self.align.is_power_of_two());
        if self.align % PAGE_SIZE != 0 || !self.align.is_power_of_two() {
            return_errno_with_message!(Errno::EINVAL, "invalid align");
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

    /// check whether the vmperm is subset of vmo rights
    fn check_perms(&self) -> Result<()> {
        let perm_rights = Rights::from(self.perms);
        self.vmo.check_rights(perm_rights)
    }

    /// check whether the vmo will overwrite with any existing vmo or vmar
    fn check_overwrite(&self) -> Result<()> {
        if self.can_overwrite {
            // if can_overwrite is set, the offset cannot be None
            debug_assert!(self.offset != None);
            if self.offset == None {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "offset can not be none when can overwrite is true"
                );
            }
        }
        if self.offset == None {
            // if does not specify the offset, we assume the map can always find suitable free region.
            // FIXME: is this always true?
            return Ok(());
        }
        let offset = self.offset.unwrap();
        // we should spare enough space at least for the whole vmo
        let size = self.size.max(self.vmo.size());
        let vmo_range = offset..(offset + size);
        self.parent
            .0
            .check_vmo_overwrite(vmo_range, self.can_overwrite)
    }
}
