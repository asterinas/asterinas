// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod static_cap;
mod vm_allocator;
pub mod vm_mapping;

use core::{
    num::NonZeroUsize,
    ops::Range,
    sync::atomic::{AtomicU32, Ordering},
};

use align_ext::AlignExt;
use aster_rights::Rights;
use ostd::mm::{
    tlb::TlbFlushOp,
    vm_space::{Token, VmItem, VmSpace},
    CachePolicy, FrameAllocOptions, PageFlags, PageProperty, MAX_USERSPACE_VADDR,
};
#[cfg(feature = "dist_vmar_alloc")]
use vm_allocator::PerCpuAllocator;
#[cfg(not(feature = "dist_vmar_alloc"))]
use vm_allocator::SimpleAllocator;
use vm_allocator::VmAllocator;
use vm_mapping::{VmMarker, VmoBackedVMA};

use self::vm_mapping::MappedVmo;
use super::page_fault_handler::PageFaultHandler;
use crate::{
    prelude::*,
    thread::exception::PageFaultInfo,
    vm::{
        perms::VmPerms,
        util::duplicate_frame,
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

    fn alloc_vmo_backed_id(&self) -> u32 {
        self.0.vmo_backed_id_alloc.fetch_add(1, Ordering::Relaxed)
    }
}

pub(super) struct Vmar_ {
    /// The offset relative to the root VMAR
    base: Vaddr,
    /// The total size of the VMAR in bytes
    size: usize,
    /// The attached `VmSpace`
    vm_space: Arc<VmSpace>,

    #[cfg(feature = "dist_vmar_alloc")]
    allocator: PerCpuAllocator,
    #[cfg(not(feature = "dist_vmar_alloc"))]
    allocator: SimpleAllocator,

    vmo_backed_id_alloc: AtomicU32,
    /// The map from the VMO-backed ID to the VMA structure.
    vma_map: RwLock<BTreeMap<u32, VmoBackedVMA>>,
}

pub const ROOT_VMAR_LOWEST_ADDR: Vaddr = 0x001_0000; // 64 KiB is the Linux configurable default
pub const ROOT_VMAR_GROWUP_BASE: Vaddr = (MAX_USERSPACE_VADDR + PAGE_SIZE) / 16;
const ROOT_VMAR_CAP_ADDR: Vaddr = MAX_USERSPACE_VADDR;

/// Returns whether the input `vaddr` is a legal user space virtual address.
pub fn is_userspace_vaddr(vaddr: Vaddr) -> bool {
    (ROOT_VMAR_LOWEST_ADDR..ROOT_VMAR_CAP_ADDR).contains(&vaddr)
}

impl Vmar_ {
    fn alloc_growup_region(&self, size: usize, align: usize) -> Result<usize> {
        self.allocator.allocate(size, align)
    }

    fn allocate_fixed(&self, start: usize, end: usize) {
        self.allocator.allocate_fixed(start, end);
    }

    fn new_root() -> Arc<Self> {
        let vm_space = VmSpace::new();
        let vm_space = Arc::new(vm_space);

        Arc::new(Vmar_ {
            base: ROOT_VMAR_LOWEST_ADDR,
            size: ROOT_VMAR_CAP_ADDR - ROOT_VMAR_LOWEST_ADDR,
            vm_space,

            #[cfg(feature = "dist_vmar_alloc")]
            allocator: PerCpuAllocator::new(),
            #[cfg(not(feature = "dist_vmar_alloc"))]
            allocator: SimpleAllocator::new(),

            vmo_backed_id_alloc: AtomicU32::new(1),
            vma_map: RwLock::new(BTreeMap::new()),
        })
    }

    fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        assert!(range.start % PAGE_SIZE == 0);
        assert!(range.end % PAGE_SIZE == 0);

        let mut cursor = self.vm_space.cursor_mut(&range).unwrap();

        let mut prot_op = |p: &mut PageProperty| {
            let mut cow = p.flags.contains(PageFlags::AVAIL1);
            let shared = p.flags.contains(PageFlags::AVAIL2);
            // If protect COW read-only mapping to read, remove COW flag.
            if cow && !perms.contains(VmPerms::WRITE) {
                cow = false;
            }
            // If protect private read-only mapping to write, perform COW.
            if !shared && perms.contains(VmPerms::WRITE) {
                cow = true;
            }
            p.flags = perms.into();
            if cow {
                p.flags |= PageFlags::AVAIL1;
            }
            if shared {
                p.flags |= PageFlags::AVAIL2;
            }
        };
        let mut token_op = |t: &mut Token| {
            let mut marker = VmMarker::decode(*t);
            marker.perms = perms;
            *t = marker.encode();
        };
        while cursor.virt_addr() < range.end {
            if let Some(va) =
                cursor.protect_next(range.end - cursor.virt_addr(), &mut prot_op, &mut token_op)
            {
                cursor.flusher().issue_tlb_flush(TlbFlushOp::Range(va));
            } else {
                break;
            }
        }
        cursor.flusher().dispatch_tlb_flush();
        cursor.flusher().sync_tlb_flush();

        Ok(())
    }

    /// Handles user space page fault, if the page fault is successfully handled, return Ok(()).
    pub fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        let address = page_fault_info.address;

        log::trace!(
            "page fault at address 0x{:x}, perms: {:?}",
            address,
            page_fault_info.required_perms
        );

        if !(self.base..self.base + self.size).contains(&address) {
            return_errno_with_message!(Errno::EACCES, "page fault address is not in current VMAR");
        }

        let page_aligned_addr = address.align_down(PAGE_SIZE);
        let is_write_fault = page_fault_info.required_perms.contains(VmPerms::WRITE);
        let is_exec_fault = page_fault_info.required_perms.contains(VmPerms::EXEC);

        let mut cursor = self
            .vm_space
            .cursor_mut(&(page_aligned_addr..page_aligned_addr + PAGE_SIZE))?;

        match cursor.query().unwrap() {
            VmItem::Marked {
                va: _,
                len: _,
                token,
            } => {
                let marker = VmMarker::decode(token);

                if !marker.perms.contains(page_fault_info.required_perms) {
                    trace!(
                        "self.perms {:?}, page_fault_info.required_perms {:?}",
                        marker.perms,
                        page_fault_info.required_perms,
                    );
                    return_errno_with_message!(Errno::EACCES, "perm check fails");
                }

                if let Some(vmo_backed_id) = marker.vmo_backed_id {
                    // On-demand VMO-backed mapping.
                    //
                    // It includes file-backed mapping and shared anonymous mapping.

                    let id_map = self.vma_map.read();
                    let vmo_backed_vma = id_map.get(&vmo_backed_id).unwrap();
                    let vmo = &vmo_backed_vma.vmo;

                    let (frame, need_cow) = {
                        let page_offset =
                            address.align_down(PAGE_SIZE) - vmo_backed_vma.map_to_addr;
                        if let Ok(frame) = vmo.get_committed_frame(page_offset) {
                            if !marker.is_shared && is_write_fault {
                                // Write access to private VMO-backed mapping. Performs COW directly.
                                (duplicate_frame(&frame)?.into(), false)
                            } else {
                                // Operations to shared mapping or read access to private VMO-backed mapping.
                                // If read access to private VMO-backed mapping triggers a page fault,
                                // the map should be readonly. If user next tries to write to the frame,
                                // another page fault will be triggered which will performs a COW (Copy-On-Write).
                                (frame, !marker.is_shared)
                            }
                        } else if !marker.is_shared {
                            // The page index is outside the VMO. This is only allowed in private mapping.
                            (FrameAllocOptions::new().alloc_frame()?.into(), false)
                        } else {
                            return_errno_with_message!(
                                Errno::EFAULT,
                                "could not find a corresponding physical page"
                            );
                        }
                    };

                    let mut page_flags = marker.perms.into();

                    if need_cow {
                        page_flags -= PageFlags::W;
                        page_flags |= PageFlags::AVAIL1;
                    }

                    if marker.is_shared {
                        page_flags |= PageFlags::AVAIL2;
                    }

                    // Pre-fill A/D bits to avoid A/D TLB miss.
                    page_flags |= PageFlags::ACCESSED;
                    if is_write_fault {
                        page_flags |= PageFlags::DIRTY;
                    }
                    let map_prop = PageProperty::new(page_flags, CachePolicy::Writeback);

                    cursor.map(frame, map_prop);
                } else {
                    // On-demand non-vmo-backed mapping.
                    //
                    // It is a private anonymous mapping.

                    let vm_perms = marker.perms;

                    let mut page_flags = vm_perms.into();

                    if marker.is_shared {
                        page_flags |= PageFlags::AVAIL2;
                        unimplemented!("shared non-vmo-backed mapping");
                    }

                    // Pre-fill A/D bits to avoid A/D TLB miss.
                    page_flags |= PageFlags::ACCESSED;
                    if is_write_fault {
                        page_flags |= PageFlags::DIRTY;
                    }

                    let map_prop = PageProperty::new(page_flags, CachePolicy::Writeback);

                    cursor.map(FrameAllocOptions::new().alloc_frame()?.into(), map_prop);
                }
            }
            VmItem::Mapped {
                va,
                frame,
                mut prop,
            } => {
                if VmPerms::from(prop.flags).contains(page_fault_info.required_perms) {
                    // The page fault is already handled maybe by other threads.
                    // Just flush the TLB and return.
                    TlbFlushOp::Address(va).perform_on_current();
                    return Ok(());
                }

                if is_exec_fault {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "page fault at non-executable mapping"
                    );
                }

                let is_cow = prop.flags.contains(PageFlags::AVAIL1);
                let is_shared = prop.flags.contains(PageFlags::AVAIL2);

                if !is_cow && is_write_fault {
                    return_errno_with_message!(Errno::EACCES, "page fault at read-only mapping");
                }

                // Perform COW if it is a write access to a shared mapping.

                // If the forked child or parent immediately unmaps the page after
                // the fork without accessing it, we are the only reference to the
                // frame. We can directly map the frame as writable without
                // copying. In this case, the reference count of the frame is 2 (
                // one for the mapping and one for the frame handle itself).
                let only_reference = frame.reference_count() == 2;

                let additional_flags = PageFlags::W | PageFlags::ACCESSED | PageFlags::DIRTY;

                if is_shared || only_reference {
                    cursor.protect_next(
                        PAGE_SIZE,
                        &mut |p: &mut PageProperty| {
                            p.flags |= additional_flags;
                            p.flags -= PageFlags::AVAIL1; // Remove COW flag
                        },
                        &mut |_: &mut Token| {},
                    );
                    cursor.flusher().issue_tlb_flush(TlbFlushOp::Address(va));
                } else {
                    let new_frame = duplicate_frame(&frame)?;
                    prop.flags |= additional_flags;
                    cursor.map(new_frame.into(), prop);
                }
            }
            VmItem::NotMapped { .. } => {
                return_errno_with_message!(Errno::EACCES, "page fault at an address not mapped");
            }
        }

        Ok(())
    }

    /// Clears all content of the root VMAR.
    fn clear_root_vmar(&self) -> Result<()> {
        let full_range = 0..MAX_USERSPACE_VADDR;
        let mut cursor = self.vm_space.cursor_mut(&full_range).unwrap();
        cursor.unmap(full_range.len());

        self.allocator.clear();
        self.vmo_backed_id_alloc.store(1, Ordering::Release);
        self.vma_map.write().clear();

        cursor.flusher().sync_tlb_flush();
        Ok(())
    }

    pub fn remove_mapping(&self, range: Range<usize>) -> Result<()> {
        let mut cursor = self.vm_space.cursor_mut(&range).unwrap();
        cursor.unmap(range.len());
        cursor.flusher().sync_tlb_flush();

        Ok(())
    }

    /// Returns the attached `VmSpace`.
    fn vm_space(&self) -> &Arc<VmSpace> {
        &self.vm_space
    }

    pub(super) fn new_fork_root(self: &Arc<Self>) -> Result<Arc<Self>> {
        // Clone mappings.
        let new_vmspace = VmSpace::new();

        let range = self.base..(self.base + self.size);
        let mut new_cursor = new_vmspace.cursor_mut(&range).unwrap();
        let cur_vmspace = self.vm_space();
        let mut cur_cursor = cur_vmspace.cursor_mut(&range).unwrap();

        let old_vma_map = self.vma_map.read();
        let mut new_vma_map = BTreeMap::new();

        // Protect the mapping and copy to the new page table for COW.
        let mut prot_op = |page: &mut PageProperty| {
            if page.flags.contains(PageFlags::W) {
                page.flags |= PageFlags::AVAIL1; // Copy-on-write
            }
            page.flags -= PageFlags::W;
        };
        let mut token_op = |token: &mut Token| {
            let marker = VmMarker::decode(*token);
            if let Some(vmo_backed_id) = marker.vmo_backed_id {
                new_vma_map
                    .entry(vmo_backed_id)
                    .or_insert_with(|| old_vma_map.get(&vmo_backed_id).unwrap().clone());
            }
        };
        new_cursor.copy_from(&mut cur_cursor, range.len(), &mut prot_op, &mut token_op);

        let new_vmo_backed_id_alloc = self.vmo_backed_id_alloc.load(Ordering::Acquire);

        cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::All);
        cur_cursor.flusher().dispatch_tlb_flush();
        cur_cursor.flusher().sync_tlb_flush();
        drop(new_cursor);
        drop(cur_cursor);

        Ok(Arc::new(Self {
            base: self.base,
            size: self.size,
            vm_space: Arc::new(new_vmspace),
            allocator: self.allocator.fork(),
            vmo_backed_id_alloc: AtomicU32::new(new_vmo_backed_id_alloc),
            vma_map: RwLock::new(new_vma_map),
        }))
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

        // Allocates a free region.
        trace!("allocate free region, map_size = 0x{:x}, offset = {:x?}, align = 0x{:x}, can_overwrite = {}", map_size, offset, align, can_overwrite);
        let map_to_addr = if can_overwrite {
            // If can overwrite, the offset is ensured not to be `None`.
            let offset = offset.ok_or(Error::with_message(
                Errno::EINVAL,
                "offset cannot be None since can overwrite is set",
            ))?;
            if offset + map_size > ROOT_VMAR_GROWUP_BASE {
                parent.0.allocate_fixed(offset, offset + map_size);
            }
            offset
        } else if let Some(offset) = offset {
            if offset + map_size > ROOT_VMAR_GROWUP_BASE {
                panic!("Exact allocation exceeds the root VMAR's upper bound");
            }
            offset
        } else {
            parent.0.alloc_growup_region(map_size, align)?
        };

        let vmo = vmo.map(|vmo| MappedVmo::new(vmo.to_dyn(), vmo_offset..vmo_limit));

        trace!(
            "build mapping, range = {:#x?}, perms = {:?}, vmo = {:#?}",
            map_to_addr..map_to_addr + map_size,
            perms,
            vmo
        );

        // Build the mapping.

        let mut cursor = parent
            .vm_space()
            .cursor_mut(&(map_to_addr..map_to_addr + map_size))
            .unwrap();

        if !can_overwrite {
            while cursor.virt_addr() < map_to_addr + map_size {
                let item = cursor.query().unwrap();
                if let VmItem::NotMapped { va, len } = item {
                    if va + len < map_to_addr {
                        cursor.jump(va + len).unwrap();
                    } else {
                        break;
                    }
                } else {
                    return_errno_with_message!(Errno::EINVAL, "overlapping mapping");
                }
            }
        }

        let marker = VmMarker {
            perms,
            is_shared,
            vmo_backed_id: if let Some(vmo) = vmo {
                let id = parent.alloc_vmo_backed_id();
                let vma = VmoBackedVMA {
                    id,
                    map_size: NonZeroUsize::new(map_size).unwrap(),
                    map_to_addr,
                    vmo,
                    handle_page_faults_around,
                };
                let mut vma_map = parent.0.vma_map.write();
                vma_map.insert(id, vma);
                Some(id)
            } else {
                None
            },
        };

        cursor.jump(map_to_addr).unwrap();
        cursor.mark(map_size, marker.encode());

        Ok(map_to_addr)
    }

    /// Checks whether all options are valid.
    fn check_options(&self) -> Result<()> {
        // Check align.
        debug_assert!(self.align % PAGE_SIZE == 0);
        // Size cannot be zero.
        if self.size == 0 {
            return_errno_with_message!(Errno::EINVAL, "mapping size is zero");
        }
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
