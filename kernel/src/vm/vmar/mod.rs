// SPDX-License-Identifier: MPL-2.0

//! Virtual Memory Address Regions (VMARs).

mod dyn_cap;
mod static_cap;
mod vm_allocator;
pub mod vm_mapping;

use core::{
    cmp::min,
    num::NonZeroUsize,
    ops::Range,
    sync::atomic::{AtomicU32, Ordering},
};

use align_ext::AlignExt;
use aster_rights::Rights;
use ostd::{
    cpu::CpuId,
    mm::{
        tlb::TlbFlushOp,
        vm_space::{largest_pages, CursorMut, Status, VmItem, VmSpace},
        CachePolicy, Frame, FrameAllocOptions, PageFlags, PageProperty, UFrame, UntypedMem,
        MAX_USERSPACE_VADDR,
    },
    task::disable_preempt,
};
#[cfg(feature = "dist_vmar_alloc")]
use vm_allocator::PerCpuAllocator;
#[cfg(not(feature = "dist_vmar_alloc"))]
use vm_allocator::SimpleAllocator;
use vm_allocator::VmAllocator;

use self::vm_mapping::{MappedVmo, VmMarker, VmoBackedVMA};
use crate::{
    fs::utils::Inode,
    prelude::*,
    thread::exception::PageFaultInfo,
    util::per_cpu_counter::PerCpuCounter,
    vm::{
        perms::VmPerms,
        vmo::{CommitFlags, Vmo, VmoCommitError, VmoRightsOp},
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
    /// If the new mapping size is smaller than the original mapping size, the
    /// extra part will be unmapped. If the new mapping is larger than the old
    /// mapping and the extra part overlaps with existing mapping, resizing
    /// will fail and return `Err`.
    ///
    /// - When `check_single_mapping` is `true`, this method will check whether
    ///   the range of the original mapping is covered by a single [`VmMapping`].
    ///   If not, this method will return an `Err`.
    /// - When `check_single_mapping` is `false`, The range of the original
    ///   mapping does not have to solely map to a whole [`VmMapping`], but it
    ///   must ensure that all existing ranges have a mapping. Otherwise, this
    ///   method will return an `Err`.
    pub fn resize_mapping(
        &self,
        _map_addr: Vaddr,
        _old_size: usize,
        _new_size: usize,
        _check_single_mapping: bool,
    ) -> Result<()> {
        todo!();
    }

    /// Remaps the original mapping to a new address and/or size.
    ///
    /// If the new mapping size is smaller than the original mapping size, the
    /// extra part will be unmapped.
    ///
    /// - If `new_addr` is `Some(new_addr)`, this method attempts to move the
    ///   mapping from `old_addr..old_addr + old_size` to `new_addr..new_addr +
    ///   new_size`. If any existing mappings lie within the target range,
    ///   they will be unmapped before the move.
    /// - If `new_addr` is `None`, a new range of size `new_size` will be
    ///   allocated, and the original mapping will be moved there.
    pub fn remap(
        &self,
        _old_addr: Vaddr,
        _old_size: usize,
        _new_addr: Option<Vaddr>,
        _new_size: usize,
    ) -> Result<Vaddr> {
        todo!();
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

    /// Virtual memory allocator for this VMAR.
    #[cfg(feature = "dist_vmar_alloc")]
    allocator: PerCpuAllocator,
    #[cfg(not(feature = "dist_vmar_alloc"))]
    allocator: SimpleAllocator,

    /// The ID allocator for VMO mappings.
    vmo_backed_id_alloc: AtomicU32,
    /// The map from the VMO-backed ID to the VMA structure.
    vma_map: RwLock<BTreeMap<u32, VmoBackedVMA>>,
    /// The RSS counters.
    rss_counters: [PerCpuCounter; NUM_RSS_COUNTERS],
}

pub const ROOT_VMAR_LOWEST_ADDR: Vaddr = 0x001_0000; // 64 KiB is the Linux configurable default
pub const INIT_STACK_CLEARANCE: Vaddr = MAX_USERSPACE_VADDR - 0x4000_0000; // 1 GiB
pub const ROOT_VMAR_CAP_ADDR: Vaddr = MAX_USERSPACE_VADDR;

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
        let rss_counters = core::array::from_fn(|_| PerCpuCounter::new());

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

            rss_counters,
        })
    }

    fn protect(&self, perms: VmPerms, range: Range<usize>) -> Result<()> {
        assert!(range.start % PAGE_SIZE == 0);
        assert!(range.end % PAGE_SIZE == 0);

        let preempt_guard = disable_preempt();
        let mut cursor = self.vm_space.cursor_mut(&preempt_guard, &range).unwrap();

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
        let mut token_op = |t: &mut Status| {
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

        #[cfg(not(feature = "mprotect_async_tlb"))]
        {
            cursor.flusher().dispatch_tlb_flush();
            cursor.flusher().sync_tlb_flush();
        }

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

        let mut zeroed_frame = Some(FrameAllocOptions::new().alloc_frame()?);
        let mut rss_delta = RssDelta::new(self);

        'retry: loop {
            let preempt_guard = disable_preempt();
            let locked_2m_range = {
                let locked_2m_start = page_aligned_addr.align_down(PAGE_SIZE * 512);
                locked_2m_start..min(locked_2m_start + PAGE_SIZE * 512, MAX_USERSPACE_VADDR)
            };
            let mut cursor = self
                .vm_space
                .cursor_mut(&preempt_guard, &locked_2m_range)
                .unwrap();
            cursor.jump(page_aligned_addr).unwrap();

            match cursor.query().unwrap() {
                (_, Some(VmItem::Status(status, _))) => {
                    let marker = VmMarker::decode(status);

                    if !marker.perms.contains(page_fault_info.required_perms) {
                        trace!(
                            "self.perms {:?}, page_fault_info.required_perms {:?}",
                            marker.perms,
                            page_fault_info.required_perms,
                        );
                        return_errno_with_message!(Errno::EACCES, "perm check fails");
                    }

                    if marker.vmo_backed_id.is_some() {
                        let id_map = self.vma_map.read();
                        let pf_fault_vmo_id = marker.vmo_backed_id.unwrap();
                        let vmo_backed_vma = id_map.get(&pf_fault_vmo_id).unwrap();

                        let res = self.handle_file_backed_pf_under_cursor(
                            &mut zeroed_frame,
                            &mut cursor,
                            &mut rss_delta,
                            marker,
                            vmo_backed_vma,
                            page_fault_info,
                        );
                        match res {
                            Err(VmoCommitError::NeedIo(index)) => {
                                drop(cursor);
                                drop(preempt_guard);
                                vmo_backed_vma.vmo.commit_on(index, CommitFlags::empty())?;
                                continue 'retry;
                            }
                            Err(VmoCommitError::Err(e)) => {
                                return Err(e);
                            }
                            _ => {}
                        }
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

                        let map_prop = PageProperty::new_user(page_flags, CachePolicy::Writeback);

                        rss_delta.add(RssType::RSS_ANONPAGES, 1);

                        cursor.map(VmItem::Frame(zeroed_frame.take().unwrap().into(), map_prop));
                    }
                }
                (va, Some(VmItem::Frame(frame, mut prop))) => {
                    if VmPerms::from(prop.flags).contains(page_fault_info.required_perms) {
                        // The page fault is already handled maybe by other threads.
                        // Just flush the TLB and return.
                        TlbFlushOp::Range(va).perform_on_current();
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
                        return_errno_with_message!(
                            Errno::EACCES,
                            "page fault at read-only mapping"
                        );
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
                            &mut |_: &mut Status| {},
                        );
                        cursor.flusher().issue_tlb_flush(TlbFlushOp::Range(va));
                    } else {
                        let new_frame = zeroed_frame.take().unwrap();
                        new_frame.writer().write(&mut frame.reader());

                        prop.flags |= additional_flags;
                        prop.flags -= PageFlags::AVAIL1; // Remove COW flag

                        cursor.map(VmItem::Frame(new_frame.into(), prop));
                        cursor.flusher().issue_tlb_flush(TlbFlushOp::Range(va));
                    }
                    cursor.flusher().dispatch_tlb_flush();
                    cursor.flusher().sync_tlb_flush();
                }
                (_, None) => {
                    return_errno_with_message!(
                        Errno::EACCES,
                        "page fault at an address not mapped"
                    );
                }
            }
            break 'retry;
        }

        Ok(())
    }

    fn handle_file_backed_pf_under_cursor(
        &self,
        zeroed_frame: &mut Option<Frame<()>>,
        cursor: &mut CursorMut<'_>,
        rss_delta: &mut RssDelta<'_>,
        marker: VmMarker,
        vmo_backed_vma: &VmoBackedVMA,
        page_fault_info: &PageFaultInfo,
    ) -> core::result::Result<(), VmoCommitError> {
        // On-demand VMO-backed mapping.
        //
        // It includes file-backed mapping and shared anonymous mapping.

        let pf_fault_vmo_id = marker.vmo_backed_id.unwrap();
        let vmo = &vmo_backed_vma.vmo;

        let page_aligned_addr = page_fault_info.address.align_down(PAGE_SIZE);
        let is_write_fault = page_fault_info.required_perms.contains(VmPerms::WRITE);

        let vmo_map_to_addr = vmo_backed_vma.map_to_addr;
        let fault_vmo_offset = page_aligned_addr - vmo_map_to_addr;

        if !vmo_backed_vma.handle_page_faults_around {
            cursor.jump(page_aligned_addr).unwrap();
            let mut commit_fn = || vmo.get_committed_frame(fault_vmo_offset);
            return self.handle_one_file_backed_pf_under_cursor(
                cursor,
                rss_delta,
                marker,
                &mut commit_fn,
                zeroed_frame,
                is_write_fault,
            );
        }

        let fault_around_range = {
            let fault_around_start = page_aligned_addr.align_down(PAGE_SIZE * 16);
            fault_around_start..min(fault_around_start + PAGE_SIZE * 16, MAX_USERSPACE_VADDR)
        };

        let vmo_offset_end = vmo.valid_size();
        let (vmo_range, va_range) =
            if !marker.is_shared && (is_write_fault || vmo_offset_end <= fault_vmo_offset) {
                // Private out-range mapping. Perform only single page.
                (
                    fault_vmo_offset..fault_vmo_offset + PAGE_SIZE,
                    page_aligned_addr..page_aligned_addr + PAGE_SIZE,
                )
            } else {
                let (vmo_start, va_start) = if fault_around_range.start > vmo_map_to_addr {
                    (
                        fault_around_range.start - vmo_map_to_addr,
                        fault_around_range.start,
                    )
                } else {
                    (0, vmo_map_to_addr)
                };
                let vmo_end = min(vmo_offset_end, fault_around_range.end - vmo_map_to_addr);
                let va_end = min(fault_around_range.end, vmo_offset_end + vmo_map_to_addr);
                (vmo_start..vmo_end, va_start..va_end)
            };

        log::trace!(
            "Handle fb pf range under cursor, vmo_range = {:#x?}, va_range = {:#x?}",
            vmo_range,
            va_range
        );

        let mut cur_offset = vmo_range.start;
        let mut cur_va = va_range.start;

        vmo.operate_on_range(
            &vmo_range,
            move |commit_fn: &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>| {
                cursor.jump(cur_va).unwrap();

                log::trace!(
                    "Handling page fault at vaddr 0x{:x}, vmo_offset = 0x{:x}",
                    cur_va,
                    cur_offset
                );

                if let (_, Some(VmItem::Status(status, _))) = cursor.query().unwrap() {
                    let marker = VmMarker::decode(status);
                    if !marker.perms.contains(page_fault_info.required_perms) {
                        return Ok(());
                    }
                    let Some(marked_vmo_backed_id) = marker.vmo_backed_id else {
                        return Ok(());
                    };
                    if marked_vmo_backed_id != pf_fault_vmo_id {
                        return Ok(());
                    }

                    let res = self.handle_one_file_backed_pf_under_cursor(
                        cursor,
                        rss_delta,
                        marker,
                        commit_fn,
                        zeroed_frame,
                        is_write_fault,
                    );

                    if page_aligned_addr == cur_va {
                        res?;
                    }
                }

                cur_offset += PAGE_SIZE;
                cur_va += PAGE_SIZE;

                Ok(())
            },
        )
    }

    fn handle_one_file_backed_pf_under_cursor(
        &self,
        cursor: &mut CursorMut<'_>,
        rss_delta: &mut RssDelta<'_>,
        marker: VmMarker,
        commit_fn: &mut dyn FnMut() -> core::result::Result<UFrame, VmoCommitError>,
        zeroed_frame: &mut Option<Frame<()>>,
        is_write_fault: bool,
    ) -> core::result::Result<(), VmoCommitError> {
        let (frame, need_cow) = {
            let commit_res = commit_fn();
            match commit_res {
                Ok(frame) => {
                    if !marker.is_shared && is_write_fault {
                        // Write access to private VMO-backed mapping. Performs COW directly.
                        let allocated = if let Some(frame) = zeroed_frame.take() {
                            frame.into()
                        } else {
                            alloc_frame(false)?
                        };
                        allocated.writer().write(&mut frame.reader());
                        (allocated, false)
                    } else {
                        // Operations to shared mapping or read access to private VMO-backed mapping.
                        // If read access to private VMO-backed mapping triggers a page fault,
                        // the map should be readonly. If user next tries to write to the frame,
                        // another page fault will be triggered which will performs a COW (Copy-On-Write).
                        (frame, !marker.is_shared)
                    }
                }
                Err(VmoCommitError::NeedIo(index)) => {
                    return Err(VmoCommitError::NeedIo(index));
                }
                Err(_) => {
                    if !marker.is_shared {
                        // The page index is outside the VMO. This is only allowed in private mapping.
                        let frame = if let Some(frame) = zeroed_frame.take() {
                            frame.into()
                        } else {
                            alloc_frame(true)?
                        };
                        rss_delta.add(RssType::RSS_ANONPAGES, 1);
                        (frame, false)
                    } else {
                        return Err(VmoCommitError::Err(Error::with_message(
                            Errno::EFAULT,
                            "could not find a corresponding physical page",
                        )));
                    }
                }
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
        let map_prop = PageProperty::new_user(page_flags, CachePolicy::Writeback);

        rss_delta.add(RssType::RSS_FILEPAGES, 1);

        cursor.map(VmItem::Frame(frame, map_prop));

        Ok(())
    }

    /// Clears all content of the root VMAR.
    fn clear_root_vmar(&self) -> Result<()> {
        {
            let full_range = 0..MAX_USERSPACE_VADDR;
            let preempt_guard = disable_preempt();
            let mut cursor = self
                .vm_space
                .cursor_mut(&preempt_guard, &full_range)
                .unwrap();
            cursor.unmap(full_range.len());
            cursor.flusher().sync_tlb_flush();
        }

        self.allocator.clear();
        self.vmo_backed_id_alloc.store(1, Ordering::Release);
        self.vma_map.write().clear();

        Ok(())
    }

    pub fn remove_mapping(&self, range: Range<usize>) -> Result<()> {
        let preempt_guard = disable_preempt();
        let mut cursor = self.vm_space.cursor_mut(&preempt_guard, &range).unwrap();

        // let mut remain_size = range.len();
        // while let Some(mapped_va) = cursor.find_next(remain_size) {
        //     let (va, Some(item)) = cursor.query().unwrap() else {
        //         panic!("Found mapped page but query failed");
        //     };
        //     debug_assert_eq!(mapped_va, va.start);

        //     if let VmItem::Frame(frame, _) = item {
        //         // Update RSS counters.
        //         if (frame.dyn_meta() as &dyn Any)
        //             .downcast_ref::<CachePageMeta>()
        //             .is_some()
        //         {
        //             self.add_rss_counter(RssType::RSS_FILEPAGES, -1);
        //         } else {
        //             self.add_rss_counter(RssType::RSS_ANONPAGES, -1);
        //         }
        //     }
        //     if va.end < range.end {
        //         cursor.jump(va.end).unwrap();
        //         remain_size = range.end - va.end;
        //     } else {
        //         break;
        //     }
        // }

        // cursor.jump(range.start).unwrap();

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
        let mut rss_counters = [0_isize; NUM_RSS_COUNTERS];

        let (new_vma_map, new_vmo_backed_id_alloc) = {
            let preempt_guard = disable_preempt();
            let range = self.base..(self.base + self.size);
            let mut new_cursor = new_vmspace.cursor_mut(&preempt_guard, &range).unwrap();
            let cur_vmspace = self.vm_space();
            let mut cur_cursor = cur_vmspace.cursor_mut(&preempt_guard, &range).unwrap();

            let old_vma_map = self.vma_map.read();
            let mut new_vma_map = BTreeMap::new();
            let mut status_op = |status: &mut Status| {
                let marker = VmMarker::decode(*status);
                if let Some(vmo_backed_id) = marker.vmo_backed_id {
                    new_vma_map
                        .entry(vmo_backed_id)
                        .or_insert_with(|| old_vma_map.get(&vmo_backed_id).unwrap().clone());
                }
            };
            cow_copy_pt(
                &mut cur_cursor,
                &mut new_cursor,
                range.len(),
                &mut status_op,
                &mut rss_counters,
            );

            cur_cursor.flusher().issue_tlb_flush(TlbFlushOp::All);
            cur_cursor.flusher().dispatch_tlb_flush();
            cur_cursor.flusher().sync_tlb_flush();

            // Clone other things under the lock.
            let new_vmo_backed_id_alloc = self.vmo_backed_id_alloc.load(Ordering::Acquire);

            (new_vma_map, new_vmo_backed_id_alloc)
        };

        let cpu = CpuId::current_racy(); // Safe because only used by per-cpu counters.

        Ok(Arc::new(Self {
            base: self.base,
            size: self.size,
            rss_counters: core::array::from_fn(|i| {
                let counter = PerCpuCounter::new();
                counter.add(cpu, rss_counters[i]);
                counter
            }),
            vm_space: Arc::new(new_vmspace),
            allocator: self.allocator.fork(),
            vmo_backed_id_alloc: AtomicU32::new(new_vmo_backed_id_alloc),
            vma_map: RwLock::new(new_vma_map),
        }))
    }

    pub fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.rss_counters[rss_type as usize].get()
    }

    fn add_rss_counter(&self, rss_type: RssType, val: isize) {
        // There are races but updating a remote counter won't cause any problems.
        let cpu_id = CpuId::current_racy();
        self.rss_counters[rss_type as usize].add(cpu_id, val);
    }
}

/// Sets mappings in the source page table as read-only to trigger COW, and
/// copies the mappings to the destination page table.
///
/// The copied range starts from `src`'s current position with the given
/// `size`. The destination range starts from `dst`'s current position.
///
/// The number of physical frames copied is returned.
fn cow_copy_pt(
    src: &mut CursorMut<'_>,
    dst: &mut CursorMut<'_>,
    size: usize,
    status_op: &mut impl FnMut(&mut Status),
    rss_delta: &mut [isize; NUM_RSS_COUNTERS],
) {
    let _ = rss_delta;

    let start_va = src.virt_addr();
    let end_va = start_va + size;
    let mut remain_size = size;

    // Protect the mapping and copy to the new page table for COW.
    let mut prot_op = |page: &mut PageProperty| {
        if page.flags.contains(PageFlags::W) {
            page.flags |= PageFlags::AVAIL1; // Copy-on-write
        }
        page.flags -= PageFlags::W;
    };

    while let Some(mapped_va) = src.find_next(remain_size) {
        let (va, Some(mut item)) = src.query().unwrap() else {
            panic!("Found mapped page but query failed");
        };
        debug_assert_eq!(mapped_va, va.start);

        src.protect_next(end_va - mapped_va, &mut prot_op, status_op)
            .unwrap();

        dst.jump(mapped_va).unwrap();

        match item {
            VmItem::Frame(_, ref mut prop) => {
                // let rss_type = if (frame.dyn_meta() as &dyn Any)
                //     .downcast_ref::<CachePageMeta>()
                //     .is_some()
                // {
                //     RssType::RSS_FILEPAGES
                // } else {
                //     RssType::RSS_ANONPAGES
                // };
                // rss_delta[rss_type as usize] += 1;

                prot_op(prop);
            }
            VmItem::Status(ref mut status, _) => {
                status_op(status);
            }
        }

        dst.map(item);

        remain_size = end_va - src.virt_addr();
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

    /// Returns the current RSS count for the given RSS type.
    pub fn get_rss_counter(&self, rss_type: RssType) -> usize {
        self.0.get_rss_counter(rss_type)
    }

    /// Returns the total size of the mappings in bytes.
    pub fn get_mappings_total_size(&self) -> usize {
        0 // Not supported yet.
    }
}

/// A guard for querying VMARs.
pub struct VmarQueryGuard<'a>(core::marker::PhantomData<&'a ()>);

impl Iterator for VmarQueryGuard<'_> {
    type Item = VmItem;

    fn next(&mut self) -> Option<Self::Item> {
        todo!();
    }
}

/// Options for creating a new mapping. The mapping is not allowed to overlap
/// with any child VMARs. And unless specified otherwise, it is not allowed
/// to overlap with any existing mapping, either.
pub struct VmarMapOptions<'a, R1, R2> {
    parent: &'a Vmar<R1>,
    vmo: Option<Vmo<R2>>,
    inode: Option<Arc<dyn Inode>>,
    perms: VmPerms,
    vmo_offset: usize,
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
            inode: None,
            perms,
            vmo_offset: 0,
            size,
            offset: None,
            align: PAGE_SIZE,
            can_overwrite: false,
            is_shared: false,
            handle_page_faults_around: false,
        }
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
    /// The [`Vmo`] of a mapping will be implicitly set if [`Self::inode`] is
    /// set.
    ///
    /// # Panics
    ///
    /// This function panics if an [`Inode`] is already provided.
    pub fn vmo(mut self, vmo: Vmo<R2>) -> Self {
        if self.inode.is_some() {
            panic!("Cannot set `vmo` when `inode` is already set");
        }
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

impl<R1> VmarMapOptions<'_, R1, Rights> {
    /// Binds an [`Inode`] to the mapping.
    ///
    /// This is used for file-backed mappings. The provided file inode will be
    /// mapped. See [`Self::vmo`] for details on the map size.
    ///
    /// If an [`Inode`] is provided, the [`Self::vmo`] must not be provided
    /// again. The actually mapped [`Vmo`] will be the [`Inode`]'s page cache.
    ///
    /// # Panics
    ///
    /// This function panics if:
    ///  - a [`Vmo`] or [`Inode`] is already provided;
    ///  - the provided [`Inode`] does not have a page cache.
    pub fn inode(mut self, inode: Arc<dyn Inode>) -> Self {
        if self.vmo.is_some() {
            panic!("Cannot set `inode` when `vmo` is already set");
        }
        self.vmo = Some(
            inode
                .page_cache()
                .expect("Map an inode without page cache")
                .to_dyn(),
        );
        self.inode = Some(inode);

        self
    }
}

impl<R1, R2> VmarMapOptions<'_, R1, R2>
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
            inode,
            perms,
            vmo_offset,
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
            parent.0.allocate_fixed(offset, offset + map_size);
            offset
        } else if let Some(offset) = offset {
            parent.0.allocate_fixed(offset, offset + map_size);
            offset
        } else {
            parent.0.alloc_growup_region(map_size, align)?
        };

        let vmo = vmo.map(|vmo| MappedVmo::new(vmo.to_dyn(), vmo_offset));

        trace!(
            "build mapping, range = {:#x?}, perms = {:?}, vmo = {:#?}",
            map_to_addr..map_to_addr + map_size,
            perms,
            vmo
        );

        // Build the mapping.
        let preempt_guard = disable_preempt();
        let mut cursor = parent
            .vm_space()
            .cursor_mut(&preempt_guard, &(map_to_addr..map_to_addr + map_size))
            .unwrap();

        if !can_overwrite {
            while cursor.virt_addr() < map_to_addr + map_size {
                let (va, item) = cursor.query().unwrap();
                if item.is_none() {
                    if va.end < map_to_addr {
                        cursor.jump(va.end).unwrap();
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
                    inode,
                    is_shared,
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
        let status = marker.encode();
        for item in largest_pages(map_to_addr, map_size, status) {
            cursor.map(item);
        }

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

/// The type representing categories of Resident Set Size (RSS).
///
/// See <https://github.com/torvalds/linux/blob/fac04efc5c793dccbd07e2d59af9f90b7fc0dca4/include/linux/mm_types_task.h#L26..L32>
#[repr(u32)]
#[expect(non_camel_case_types)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum RssType {
    RSS_FILEPAGES = 0,
    RSS_ANONPAGES = 1,
}

const NUM_RSS_COUNTERS: usize = 2;

pub(super) struct RssDelta<'a> {
    delta: [isize; NUM_RSS_COUNTERS],
    operated_vmar: &'a Vmar_,
}

impl<'a> RssDelta<'a> {
    pub(self) fn new(operated_vmar: &'a Vmar_) -> Self {
        Self {
            delta: [0; NUM_RSS_COUNTERS],
            operated_vmar,
        }
    }

    pub(self) fn add(&mut self, rss_type: RssType, increment: isize) {
        self.delta[rss_type as usize] += increment;
    }

    fn get(&self, rss_type: RssType) -> isize {
        self.delta[rss_type as usize]
    }
}

impl Drop for RssDelta<'_> {
    fn drop(&mut self) {
        for i in 0..NUM_RSS_COUNTERS {
            let rss_type = RssType::try_from(i as u32).unwrap();
            let delta = self.get(rss_type);
            self.operated_vmar.add_rss_counter(rss_type, delta);
        }
    }
}

fn alloc_frame(zeroed: bool) -> Result<UFrame> {
    FrameAllocOptions::new()
        .zeroed(zeroed)
        .alloc_frame()
        .map(|frame| frame.into())
        .map_err(|_| Error::with_message(Errno::ENOMEM, "failed to allocate frame"))
}

#[cfg(ktest)]
mod test {
    use ostd::{
        mm::{vm_space::VmItem, CachePolicy, FrameAllocOptions},
        prelude::*,
    };

    use super::*;

    #[ktest]
    fn test_cow_copy_pt() {
        fn status_op(_: &mut Status) {
            // No-op for status
        }

        let vm_space = VmSpace::new();
        let map_range = PAGE_SIZE..(PAGE_SIZE * 2);
        let cow_range = 0..PAGE_SIZE * 512 * 512;
        let page_property = PageProperty::new_user(PageFlags::RW, CachePolicy::Writeback);
        let preempt_guard = disable_preempt();

        // Allocates and maps a frame.
        let frame = FrameAllocOptions::default().alloc_frame().unwrap();
        let start_paddr = frame.start_paddr();
        let frame_clone_for_assert = frame.clone();

        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(VmItem::Frame(frame.into(), page_property)); // Original frame moved here

        // Confirms the initial mapping.
        assert!(matches!(
            vm_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmItem::Frame(frame, prop))) if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::RW
        ));

        // Creates a child page table with copy-on-write protection.
        let child_space = VmSpace::new();
        {
            let mut child_cursor = child_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut rss_delta = [0; NUM_RSS_COUNTERS];
            cow_copy_pt(
                &mut parent_cursor,
                &mut child_cursor,
                cow_range.len(),
                &mut status_op,
                &mut rss_delta,
            );
            assert_eq!(rss_delta[RssType::RSS_ANONPAGES as usize], 1); // Only one page should be copied
            assert_eq!(rss_delta[RssType::RSS_FILEPAGES as usize], 0); // No file pages copied
        };

        // Confirms that parent and child VAs map to the same physical address.
        {
            let child_map_frame_addr = {
                let (_, Some(VmItem::Frame(frame, _))) = child_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Child mapping query failed");
                };
                frame.start_paddr()
            };
            let parent_map_frame_addr = {
                let (_, Some(VmItem::Frame(frame, _))) = vm_space
                    .cursor(&preempt_guard, &map_range)
                    .unwrap()
                    .query()
                    .unwrap()
                else {
                    panic!("Parent mapping query failed");
                };
                frame.start_paddr()
            };
            assert_eq!(child_map_frame_addr, parent_map_frame_addr);
            assert_eq!(child_map_frame_addr, start_paddr);
        }

        // Unmaps the range from the parent.
        vm_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Confirms that the child VA remains mapped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmItem::Frame(frame, prop)))  if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::R
        ));

        // Creates a sibling page table (from the now-modified parent).
        let sibling_space = VmSpace::new();
        {
            let mut sibling_cursor = sibling_space
                .cursor_mut(&preempt_guard, &cow_range)
                .unwrap();
            let mut parent_cursor = vm_space.cursor_mut(&preempt_guard, &cow_range).unwrap();
            let mut rss_delta = [0; NUM_RSS_COUNTERS];
            cow_copy_pt(
                &mut parent_cursor,
                &mut sibling_cursor,
                cow_range.len(),
                &mut status_op,
                &mut rss_delta,
            );
            // No pages should be copied
            rss_delta.iter().for_each(|&x| assert_eq!(x, 0));
        }

        // Verifies that the sibling is unmapped as it was created after the parent unmapped the range.
        assert!(matches!(
            sibling_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));

        // Drops the parent page table.
        drop(vm_space);

        // Confirms that the child VA remains mapped after the parent is dropped.
        assert!(matches!(
            child_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmItem::Frame(frame, prop)))  if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::R
        ));

        // Unmaps the range from the child.
        child_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .unmap(map_range.len());

        // Maps the range in the sibling using the third clone.
        sibling_space
            .cursor_mut(&preempt_guard, &map_range)
            .unwrap()
            .map(VmItem::Frame(frame_clone_for_assert.into(), page_property));

        // Confirms that the sibling mapping points back to the original frame's physical address.
        assert!(matches!(
            sibling_space.cursor(&preempt_guard, &map_range).unwrap().query().unwrap(),
            (va, Some(VmItem::Frame(frame, prop)))  if va.start == map_range.start && frame.start_paddr() == start_paddr && prop.flags == PageFlags::RW
        ));

        // Confirms that the child remains unmapped.
        assert!(matches!(
            child_space
                .cursor(&preempt_guard, &map_range)
                .unwrap()
                .query()
                .unwrap(),
            (_, None)
        ));
    }
}
