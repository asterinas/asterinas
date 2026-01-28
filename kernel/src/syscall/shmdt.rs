// SPDX-License-Identifier: MPL-2.0

//! This mod defines the handler to syscall shmdt

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::shared_mem::SHM_OBJ_MANAGER};

pub fn sys_shmdt(addr: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("[sys_shmdt] addr = {:#x}", addr);

    if !addr.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "shmdt addr must be page-aligned");
    }

    let user_space = ctx.user_space();
    let root_vmar = user_space.vmar();
    let manager = SHM_OBJ_MANAGER.get().ok_or(Error::new(Errno::EINVAL))?;

    // Two-phase detachment algorithm for SysV shared memory segments.
    //
    // Since shared memory mappings can be managed with other interfaces, such
    // as `mmap`, there may be several `holes` within the shared memory
    // segment's address range. Therefore, we need a two-phase detachment
    // algorithm to first locate the target shared memory instance and then
    // unmap all VMAs belonging to that instance.

    // First pass: Starting from `addr`, scan within a heuristic-bounded window to locate a shared memory VMA.
    //
    // The scan window size is set to the maximum shared memory segment length
    // to avoid traversing the entire address space. If multiple instances
    // match, the first one found would be selected.
    let max_scan_size = { manager.read().max_shm_size() };
    if max_scan_size == 0 {
        return_errno!(Errno::EINVAL);
    }
    let scan_len = max_scan_size.align_up(PAGE_SIZE);
    addr.checked_add(scan_len)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "overflow in shmdt scan range"))?;
    let (target, seg_size) = root_vmar.find_first_shm_in_range(addr, scan_len)?;

    let end = addr
        .checked_add(seg_size)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "overflow in shmdt len"))?;
    let detach_range = addr..end;

    // Second pass: Using the instance size, collect all mappings in `[addr, addr + seg_size)` that belong to the same shared-memory attachment instance.
    //
    // We identify the instance by matching `AttachedShm`, which uniquely corresponds to a single `shmat` call. Mappings backed by the same segment
    // but created by a different `shmat` are ignored. Then unmap each VMA one
    // by one. This guarantees full cleanup even if one `shmat` creates
    // multiple non-contiguous VMAs.
    let detach_ranges = root_vmar.collect_attached_shm_ranges(detach_range, target);
    for r in detach_ranges {
        root_vmar.remove_mapping(r)?;
    }

    Ok(SyscallReturn::Return(0))
}
