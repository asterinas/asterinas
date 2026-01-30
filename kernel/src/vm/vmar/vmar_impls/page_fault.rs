// SPDX-License-Identifier: MPL-2.0

use core::{cmp::min, ops::Range};

use align_ext::AlignExt;
use ostd::{
    mm::{
        CachePolicy, Frame, FrameAllocOptions, PAGE_SIZE, PageFlags, PageProperty, UFrame, Vaddr,
        io_util::HasVmReaderWriter, page_size_at, tlb::TlbFlushOp, vm_space::VmQueriedItem,
    },
    task::disable_preempt,
};

use super::{MappedMemory, PteRangeMeta, RsAsDelta, VmMapping, Vmar, VmarCursorMut};
use crate::{
    error::{Errno, Error},
    prelude::*,
    return_errno_with_message,
    thread::exception::PageFaultInfo,
    vm::{
        perms::VmPerms,
        vmar::{interval_set::Interval, userspace_range, vm_mapping::VmoCommitHandle},
        vmo::{CommitFlags, VmoCommitError},
    },
};

#[derive(Debug)]
enum PfError {
    /// Needs IO by committing the page in [`crate::vm::vmo::Vmo`].
    Retry {
        va: Vaddr,
        how_to: VmoCommitHandle,
    },
    HardError(Error),
}

impl Vmar {
    /// Handles a page fault.
    pub fn handle_page_fault(&self, page_fault_info: &PageFaultInfo) -> Result<()> {
        let address = page_fault_info.address;

        if !userspace_range().contains(&address) {
            return_errno_with_message!(
                Errno::EFAULT,
                "handle_page_fault: page fault address is not in userspace range"
            );
        }

        // Lock the entire level-1 PT so that we can handle page faults around.
        let range_start = address.align_down(page_size_at(2));
        let range_to_lock = range_start
            ..range_start
                .checked_add(page_size_at(2))
                .unwrap_or_else(|| userspace_range().end);

        // Retry is needed if the page fault needs I/O after we locked the page table.
        // Here we declare the state visible across retries. Take care of race
        // conditions when accessing such state.
        let mut rs_as_delta = RsAsDelta::new(self);
        let mut range_handled = 0..0;

        'retry: loop {
            let preempt_guard = disable_preempt();
            let mut cursor = self.vm_space.cursor_mut(&preempt_guard, &range_to_lock)?;

            cursor.jump(address.align_down(PAGE_SIZE)).unwrap();
            while cursor.push_level_if_exists().is_some() {}

            let Some(PteRangeMeta::VmMapping(vm_mapping)) =
                cursor.aux_meta().inner.find_one(&address)
            else {
                return_errno_with_message!(
                    Errno::EACCES,
                    "handle_page_fault: no VM mappings contain the page fault address"
                );
            };

            let range_to_handle = range_to_handle(vm_mapping, page_fault_info);
            if range_handled.is_empty() {
                range_handled = range_to_handle.start..range_to_handle.start;
            }
            if range_handled.end >= range_to_handle.end {
                // We might be racing with file resizing. We must have handled
                // the faulting page since `range_to_handle` always includes it.
                debug_assert!(range_to_handle.contains(&address));
                return Ok(());
            }

            for va in (range_handled.end..range_to_handle.end).step_by(PAGE_SIZE) {
                let res = handle_one_page(&mut cursor, va, page_fault_info, &mut rs_as_delta);

                match res {
                    Ok(_) => {}
                    Err(PfError::Retry { va, how_to }) => {
                        drop(cursor);
                        drop(preempt_guard);
                        let err = how_to.commit();
                        if let Err(e) = err {
                            if va == address.align_down(PAGE_SIZE) {
                                return Err(e);
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

            cursor.flusher().dispatch_tlb_flush();
            cursor.flusher().sync_tlb_flush();

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

    let size = min(vmo.valid_size().unwrap_or(0), vm_mapping.map_size());
    let end_addr = end_addr.min(vm_mapping.map_to_addr() + size);

    // The page fault address falls outside the [`Vmo`] bounds.
    // Only a single page fault is handled in this situation.
    if end_addr <= page_addr {
        return page_addr..page_addr + PAGE_SIZE;
    }

    start_addr..end_addr
}

fn handle_one_page(
    cursor: &mut VmarCursorMut<'_>,
    va: Vaddr,
    page_fault_info: &PageFaultInfo,
    rs_as_delta: &mut RsAsDelta,
) -> core::result::Result<(), PfError> {
    cursor.jump(va).unwrap();
    while cursor.push_level_if_exists().is_some() {}

    let Some(PteRangeMeta::VmMapping(vm_mapping)) = cursor.aux_meta().inner.find_one(&va) else {
        return Err(PfError::HardError(Error::with_message(
            Errno::EACCES,
            "handle_one_page: no VM mappings contain the page fault address",
        )));
    };

    let required_perms = page_fault_info.required_perms;

    if !vm_mapping.perms().contains(page_fault_info.required_perms) {
        trace!(
            "perms {:?}, page_fault_info.required_perms {:?}, vm_mapping.range {:#x?}",
            vm_mapping.perms(),
            page_fault_info.required_perms,
            vm_mapping.range()
        );
        return Err(PfError::HardError(Error::with_message(
            Errno::EACCES,
            "perm check fails",
        )));
    }

    let is_write = required_perms.contains(VmPerms::WRITE);
    let is_shared = vm_mapping.is_shared();

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
                let new_frame = duplicate_frame(&frame).map_err(PfError::HardError)?;
                prop.flags |= new_flags;
                let _ = cursor.unmap();
                cursor.map(new_frame.into(), prop);
            }
        }
        VmQueriedItem::MappedIoMem { .. } => {
            // The page of I/O memory is populated when the memory
            // mapping is created.
            return Err(PfError::HardError(Error::with_message(
                Errno::EFAULT,
                "device memory page faults cannot be resolved",
            )));
        }
        VmQueriedItem::None => {
            // Map a new frame to the page fault address.
            let PteRangeMeta::VmMapping(vm_mapping) =
                cursor.aux_meta_mut().inner.find_one_mut(&va).unwrap()
            else {
                unreachable!("`find_one` does not stop at `VmMapping`");
            };
            let page_aligned_addr = va.align_down(PAGE_SIZE);
            let (frame, is_readonly) = match prepare_page(vm_mapping, page_aligned_addr, is_write) {
                Ok((frame, is_readonly)) => (frame, is_readonly),
                Err(VmoCommitError::Err(e)) => return Err(PfError::HardError(e)),
                Err(VmoCommitError::NeedIo(vmo_page_index)) => {
                    let how_to = vm_mapping
                        .vmo()
                        .unwrap()
                        .dup_commit(vmo_page_index, CommitFlags::empty());
                    return Err(PfError::Retry { va, how_to });
                }
            };

            let vm_perms = {
                let mut perms = vm_mapping.perms();
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

            let rss_type = vm_mapping.rss_type();
            vm_mapping.inc_frames_mapped();

            cursor.map(frame, map_prop);

            rs_as_delta.add_rs(rss_type, 1);
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

    let vmo = match vm_mapping.mapped_mem() {
        MappedMemory::Vmo(vmo) => vmo,
        MappedMemory::Anonymous => {
            // Anonymous mapping. Allocate a new frame.
            return Ok((FrameAllocOptions::new().alloc_frame()?.into(), is_readonly));
        }
        MappedMemory::Device(_) => {
            // Device memory is populated when the memory mapping is created.
            return Err(VmoCommitError::Err(Error::with_message(
                Errno::EFAULT,
                "device memory page faults cannot be resolved",
            )));
        }
    };

    let page_offset = page_aligned_addr - vm_mapping.map_to_addr();
    if !vm_mapping.is_shared() && vmo.valid_size().is_none_or(|s| page_offset >= s) {
        // The page index is outside the VMO. This is only allowed in private mapping.
        return Ok((FrameAllocOptions::new().alloc_frame()?.into(), is_readonly));
    }

    let page = vmo.get_committed_frame(page_offset)?;
    if !vm_mapping.is_shared() && write {
        // Write access to private VMO-backed mapping. Performs COW directly.
        Ok((
            duplicate_frame(&page).map_err(VmoCommitError::Err)?.into(),
            is_readonly,
        ))
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
