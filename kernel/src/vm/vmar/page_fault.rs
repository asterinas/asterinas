use core::{array, num::NonZeroUsize, ops::Range, range};

use align_ext::AlignExt;
use aster_util::per_cpu_counter::PerCpuCounter;
use ostd::{
    cpu::CpuId,
    mm::{
        CachePolicy, FrameAllocOptions, HasSize, MAX_USERSPACE_VADDR, PAGE_SIZE, PageFlags, UFrame,
        Vaddr, VmSpace,
        io_util::HasVmReaderWriter,
        page_size, page_size_at,
        tlb::TlbFlushOp,
        vm_space::{Cursor, CursorMut, VmQueriedItem},
    },
    sync::RwMutexReadGuard,
    task::{atomic_mode::AsAtomicModeGuard, disable_preempt},
};

use super::{
    Interval, IntervalSet, MappedMemory, MappedVmo, PerCpuAllocator, RssDelta, RssType, VmMapping,
    Vmar, find_next_mapped, find_next_unmappable, propagate_if_needed, split_and_insert_rest,
};
use crate::{
    error::{Errno, Error},
    fs::{file_handle::Mappable, ramfs::memfd::MemfdInode},
    process::{Process, ProcessVm, ResourceType, posix_thread::last_tid},
    return_errno_with_message,
    thread::exception::PageFaultInfo,
    vm::{
        self,
        perms::VmPerms,
        vmar::{VmarCursor, cursor_utils::unmap_count_rss},
        vmo::{CommitFlags, Vmo, VmoCommitError},
    },
};

enum PfError {
    /// Needs IO by commiting the page in [`Vmo`].
    Retry {
        va: Vaddr,
        vmo_page_index: usize,
    },
    HardError(Error),
}

impl Vmar {
    /// Handles a page fault.
    pub(super) fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<(), Error> {
        let address = page_fault_info.address;

        // Lock the entire level-1 PT so that we can handle page faults around.
        let range_to_lock = address.align_down(page_size_at(2))..address.align_up(page_size_at(2));

        // Retry is needed if the page fault needs I/O after we locked the page table.
        // Here we declare the state visible across retries. Take care of race
        // conditions when accessing such state.
        let mut rss_delta = RssDelta::new(self);
        let mut range_handled = 0..0;

        'retry: loop {
            let preempt_guard = disable_preempt();
            let mut cursor = self.vm_space.cursor_mut(&preempt_guard, &range_to_lock)?;

            let Some(vm_mapping) = cursor.aux_meta().inner.find_one(&address) else {
                return_errno_with_message!(
                    Errno::EACCES,
                    "no VM mappings contain the page fault address"
                );
            };

            let range_to_handle = range_to_handle(&vm_mapping, page_fault_info);
            if range_handled == 0..0 {
                range_handled = range_to_handle.start..range_to_handle.start;
            }
            if range_handled.end >= range_to_handle.end {
                // We might be racing with file resizing. We must have handled
                // the faulting page since `range_to_handle` always includes it.
                debug_assert!(range_to_handle.contains(&address));
                return Ok(());
            }

            for va in (range_handled.end..range_to_handle.end).step_by(PAGE_SIZE) {
                let res = handle_one_page(cursor, va, page_fault_info, &mut rss_delta);

                match res {
                    Ok(_) => {}
                    Err(PfError::Retry { va, vmo_page_index }) => {
                        let vm_mapping = cursor.aux_meta().inner.find_one(&address).unwrap();
                        let commit_handle = vm_mapping
                            .vmo()
                            .unwrap()
                            .dup_commit(vmo_page_index, CommitFlags::empty());
                        drop(cursor);
                        drop(preempt_guard);
                        let err = commit_handle.commit();
                        if err.is_err() {
                            if va == address.align_down(PAGE_SIZE) {
                                return err;
                            } else {
                                // Ignore errors caused by surrounding pages.
                                // And don't retry this page again.
                                range_handled.end = va + PAGE_SIZE;
                            }
                        }
                        continue 'retry;
                    }
                    Err(PfError::HardError(e)) => {
                        if va == address.align_down(PAGE_SIZE) {
                            return Err(e);
                        }
                    }
                }

                range_handled.end = va + PAGE_SIZE;
            }

            cursor.dispatch_tlb_flush();
            cursor.sync_tlb_flush();

            break 'retry;
        }

        Ok(())
    }
}

fn range_to_handle(vm_mapping: &VmMapping, page_fault_info: &PageFaultInfo) -> Range<Vaddr> {
    let page_addr = page_fault_info.address.align_down(PAGE_SIZE);

    if !vm_mapping.handle_page_faults_around() {
        return page_addr..page_addr + PAGE_SIZE;
    }

    const SURROUNDING_PAGE_NUM: usize = 16;
    const SURROUNDING_PAGE_ADDR_MASK: usize = !(SURROUNDING_PAGE_NUM * PAGE_SIZE - 1);

    let start_addr = (page_addr & SURROUNDING_PAGE_ADDR_MASK).max(vm_mapping.map_to_addr());
    let end_addr = (start_addr + SURROUNDING_PAGE_NUM * PAGE_SIZE).min(vm_mapping.map_end());

    let Some(vmo) = vm_mapping.vmo() else {
        return start_addr..end_addr;
    };

    let size = min(vmo.valid_size(), vm_mapping.map_size());
    let end_addr = end_addr.min(vm_mapping.map_to_addr() + size);

    // The page fault address falls outside the [`Vmo`] bounds.
    // Only a single page fault is handled in this situation.
    if end_addr <= page_addr {
        return page_addr..page_addr + PAGE_SIZE;
    }

    start_addr..end_addr
}

fn handle_one_page(
    cursor: &mut VmarCursor<'_>,
    va: Vaddr,
    page_fault_info: &PageFaultInfo,
    mut rss_delta: &mut RssDelta,
) -> Result<(), PfError> {
    let Some(vm_mapping) = cursor.aux_meta().inner.find_one(&va) else {
        return Err(PfError::HardError(Error::with_message(
            Errno::EACCES,
            "no VM mappings contain the page fault address",
        )));
    };

    if !vm_mapping.perms().contains(page_fault_info.required_perms) {
        trace!(
            "perms {:?}, page_fault_info.required_perms {:?}, vm_mapping.range {:?}",
            vm_mapping.perms(),
            page_fault_info.required_perms(),
            vm_mapping.range()
        );
        return_errno_with_message!(Errno::EACCES, "perm check fails");
    }

    let is_write = required_perms.contains(VmPerms::WRITE);
    let is_shared = vm_mapping.is_shared();
    let rss_type = vm_mapping.rss_type();

    cursor.jump(va).unwrap();
    cursor.adjust_level(1);
    let cur_va_range = cursor.cur_va_range().clone();
    debug_assert_eq!(cur_va_range, va..va + PAGE_SIZE);

    let item = cursor.query();
    match item {
        VmQueriedItem::MappedRam { frame, mut prop } => {
            if VmPerms::from(prop.flags).contains(required_perms) {
                // The page fault is already handled maybe by other threads.
                // Just flush the TLB and return.
                TlbFlushOp::for_range(cur_va_range).perform_on_current();
                return Ok(());
            }
            assert!(is_write);
            // Perform COW if it is a write access to a shared mapping.

            // Skip if the page fault is already handled.
            if prop.flags.contains(PageFlags::W) {
                return Ok(());
            }

            // If the forked child or parent immediately unmaps the page after
            // the fork without accessing it, we are the only reference to the
            // frame. We can directly map the frame as writable without copying.
            let only_reference = frame.reference_count() == 1;

            let new_flags = PageFlags::W | PageFlags::ACCESSED | PageFlags::DIRTY;

            if is_shared || only_reference {
                cursor.protect(|flags, _cache| {
                    *flags |= new_flags;
                });
                cursor
                    .flusher()
                    .issue_tlb_flush(TlbFlushOp::for_range(cur_va_range));
            } else {
                let new_frame = duplicate_frame(&frame)?;
                prop.flags |= new_flags;
                let _ = cursor.unmap();
                cursor.map(new_frame.into(), prop);
                rss_delta.add(rss_type, 1);
            }
        }
        VmQueriedItem::MappedIoMem { .. } => {
            // The page of I/O memory is populated when the memory
            // mapping is created.
            return_errno_with_message!(
                Errno::EFAULT,
                "device memory page faults cannot be resolved"
            );
        }
        VmQueriedItem::None => {
            // Map a new frame to the page fault address.
            let vm_mapping = cursor.aux_meta().inner.find_one(&va).unwrap();
            let (frame, is_readonly) = match prepare_page(vm_mapping, page_aligned_addr, is_write) {
                Ok((frame, is_readonly)) => (frame, is_readonly),
                Err(VmoCommitError::Err(e)) => return Err(PfError::HardError(e)),
                Err(VmoCommitError::NeedIo(vmo_page_index)) => {
                    return Err(PfError::Retry { va, vmo_page_index });
                }
            };

            let vm_perms = {
                let mut perms = self.perms;
                if is_readonly {
                    // COW pages are forced to be read-only.
                    perms -= VmPerms::WRITE;
                }
                perms
            };

            let mut page_flags = vm_perms.into();
            page_flags |= PageFlags::ACCESSED;
            if is_write {
                page_flags |= PageFlags::DIRTY;
            }
            let map_prop = PageProperty::new_user(page_flags, CachePolicy::Writeback);

            cursor.map(frame, map_prop);
            rss_delta.add(self.rss_type(), 1);
        }
        VmQueriedItem::PageTable => {
            unreachable!("pushed but still queried a page table")
        }
    }

    Ok(())
}

fn prepare_page(
    vm_mapping: &VmMapping,
    page_aligned_addr: Vaddr,
    write: bool,
) -> core::result::Result<(UFrame, bool), VmoCommitError> {
    let mut is_readonly = false;

    let vmo = match &vm_mapping.mapped_mem() {
        MappedMemory::Vmo(vmo) => vmo,
        MappedMemory::Anonymous => {
            // Anonymous mapping. Allocate a new frame.
            return Ok((FrameAllocOptions::new().alloc_frame()?.into(), is_readonly));
        }
        MappedMemory::Device => {
            // Device memory is populated when the memory mapping is created.
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EFAULT,
                "device memory page faults cannot be resolved",
            )));
        }
    };

    let page_offset = page_aligned_addr - vm_mapping.map_to_addr();
    if !vm_mapping.is_shared() && page_offset >= vmo.valid_size() {
        // The page index is outside the VMO. This is only allowed in private mapping.
        return Ok((FrameAllocOptions::new().alloc_frame()?.into(), is_readonly));
    }

    let page = vmo.get_committed_frame(page_offset)?;
    if !vm_mapping.is_shared() && write {
        // Write access to private VMO-backed mapping. Performs COW directly.
        Ok((duplicate_frame(&page)?.into(), is_readonly))
    } else {
        // Operations to shared mapping or read access to private VMO-backed mapping.
        // If read access to private VMO-backed mapping triggers a page fault,
        // the map should be readonly. If user next tries to write to the frame,
        // another page fault will be triggered which will performs a COW (Copy-On-Write).
        is_readonly = !vm_mapping.is_shared();
        Ok((page, is_readonly))
    }
}

fn duplicate_frame(src: &UFrame) -> Result<Frame<()>> {
    let new_frame = FrameAllocOptions::new().zeroed(false).alloc_frame()?;
    new_frame.writer().write(&mut src.reader());
    Ok(new_frame)
}
