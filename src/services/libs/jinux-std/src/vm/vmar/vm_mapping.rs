use alloc::{
    collections::{BTreeMap, BTreeSet},
    sync::{Arc, Weak},
};
use jinux_frame::{
    config::PAGE_SIZE,
    vm::{Vaddr, VmFrameVec, VmIo, VmPerm},
    Error,
};
use jinux_frame::{vm::VmMapOptions, Result};
use spin::Mutex;

use crate::vm::{page_fault_handler::PageFaultHandler, vmo::Vmo};

use super::{Vmar, Vmar_};
use crate::vm::perms::VmPerms;
use crate::vm::vmar::Rights;
use crate::vm::vmo::VmoRightsOp;

/// A VmMapping represents mapping a vmo into a vmar.
/// A vmar can has multiple VmMappings, which means multiple vmos are mapped to a vmar.
/// A vmo can also contain multiple VmMappings, which means a vmo can be mapped to multiple vmars.
/// The reltionship between Vmar and Vmo is M:N.
pub struct VmMapping {
    /// The parent vmar. The parent should always point to a valid vmar.
    parent: Weak<Vmar_>,
    /// The mapped vmo. The mapped vmo is with dynamic capability.
    vmo: Vmo<Rights>,
    /// The mao offset of the vmo, in bytes.
    vmo_offset: usize,
    /// The size of mapping, in bytes. The map size can even be larger than the size of backup vmo.
    /// Those pages outside vmo range cannot be read or write.
    map_size: usize,
    /// The base address relative to the root vmar where the vmo is mapped.
    map_to_addr: Vaddr,
    /// The pages already mapped. The key is the page index in vmo.
    mapped_pages: Mutex<BTreeSet<usize>>,
    /// The map option of each **unmapped** page. The key is the page index in vmo.
    /// This map can be filled when mapping a vmo to vmar and can be modified when call mprotect.
    /// We keep the options in case the page is not committed and will further need these options.
    page_map_options: Mutex<BTreeMap<usize, VmMapOptions>>,
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

        let real_map_size = size.min(vmo_size);
        let start_page_idx = vmo_offset / PAGE_SIZE;
        let end_page_idx = (vmo_offset + real_map_size) / PAGE_SIZE;
        let vm_space = parent_vmar.vm_space();

        let mut page_map_options = BTreeMap::new();
        let mut mapped_pages = BTreeSet::new();
        let perm = VmPerm::from(perms);
        for page_idx in start_page_idx..end_page_idx {
            let mut vm_map_options = VmMapOptions::new();
            let page_map_addr = map_to_addr + (page_idx - start_page_idx) * PAGE_SIZE;
            vm_map_options.addr(Some(page_map_addr));
            vm_map_options.perm(perm);
            vm_map_options.can_overwrite(can_overwrite);
            vm_map_options.align(align);
            if vmo.page_commited(page_idx) {
                vmo.map_page(page_idx, &vm_space, vm_map_options)?;
                mapped_pages.insert(page_idx);
            } else {
                // The page is not committed. We simple record the map options for further mapping.
                page_map_options.insert(page_idx, vm_map_options);
            }
        }
        Ok(Self {
            parent: Arc::downgrade(&parent_vmar),
            vmo,
            vmo_offset,
            map_size: size,
            map_to_addr,
            mapped_pages: Mutex::new(mapped_pages),
            page_map_options: Mutex::new(page_map_options),
        })
    }

    /// Add a new committed page and map it to vmspace
    pub fn map_one_page(&self, page_idx: usize, frames: VmFrameVec) -> Result<()> {
        let parent = self.parent.upgrade().unwrap();
        let vm_space = parent.vm_space();
        let map_addr = page_idx * PAGE_SIZE + self.map_to_addr;
        let page_map_options_lock = self.page_map_options.lock();
        let map_options = page_map_options_lock.get(&page_idx).unwrap();
        vm_space.map(frames, &map_options)?;
        self.mapped_pages.lock().insert(page_idx);
        Ok(())
    }

    pub fn unmap_one_page(&self, page_idx: usize) -> Result<()> {
        let parent = self.parent.upgrade().unwrap();
        let vm_space = parent.vm_space();
        let map_addr = page_idx * PAGE_SIZE + self.map_to_addr;
        let range = map_addr..(map_addr + PAGE_SIZE);
        vm_space.unmap(&range)?;
        self.mapped_pages.lock().remove(&page_idx);
        Ok(())
    }

    pub fn map_to_addr(&self) -> Vaddr {
        self.map_to_addr
    }

    pub fn size(&self) -> usize {
        self.map_size
    }

    pub fn read_bytes(&self, offset: usize, buf: &mut [u8]) -> Result<()> {
        let vmo_read_offset = self.vmo_offset + offset;
        self.vmo.read_bytes(vmo_read_offset, buf)
    }

    pub fn write_bytes(&self, offset: usize, buf: &[u8]) -> Result<()> {
        let vmo_write_offset = self.vmo_offset + offset;
        self.vmo.write_bytes(vmo_write_offset, buf)
    }

    pub fn handle_page_fault(&self, page_fault_addr: Vaddr, write: bool) -> Result<()> {
        let vmo_offset = self.vmo_offset + page_fault_addr - self.map_to_addr;
        self.vmo.handle_page_fault(vmo_offset, write)
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
        vmo_.add_mapping(Arc::downgrade(&vm_mapping));
        parent_vmar.add_mapping(vm_mapping);
        Ok(map_to_addr)
    }

    /// check whether all options are valid
    fn check_options(&self) -> Result<()> {
        // check align
        debug_assert!(self.align % PAGE_SIZE == 0);
        debug_assert!(self.align.is_power_of_two());
        if self.align % PAGE_SIZE != 0 || !self.align.is_power_of_two() {
            return Err(Error::InvalidArgs);
        }
        debug_assert!(self.vmo_offset % self.align == 0);
        if self.vmo_offset % self.align != 0 {
            return Err(Error::InvalidArgs);
        }
        if let Some(offset) = self.offset {
            debug_assert!(offset % self.align == 0);
            if offset % self.align != 0 {
                return Err(Error::InvalidArgs);
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
                return Err(Error::InvalidArgs);
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
