// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;
use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{prelude::*, vm::vmar::VMAR_CAP_ADDR};

pub fn sys_mincore(addr: Vaddr, len: usize, vec: Vaddr, ctx: &Context) -> Result<SyscallReturn> {
    debug!("addr = 0x{:x}, len = {}, vec = 0x{:x}", addr, len, vec);

    if !addr.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "the mapping address is not aligned");
    }
    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if VMAR_CAP_ADDR.checked_sub(addr).is_none_or(|gap| gap < len) {
        // FIXME: Linux returns `ENOMEM` if `(addr + len).align_up(PAGE_SIZE)` overflows. Here, we
        // perform a stricter validation.
        return_errno_with_message!(Errno::ENOMEM, "the mapping range is not in userspace");
    }
    let addr_range = addr..(addr + len).align_up(PAGE_SIZE);

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();

    let query_guard = vmar.query(addr_range.clone());
    if !query_guard.is_fully_mapped() {
        return_errno_with_message!(
            Errno::ENOMEM,
            "the range contains pages that are not mapped"
        );
    }

    // Stream the result in fixed-size chunks rather than allocating a single
    // buffer sized by the user-controlled range.
    const CHUNK_PAGES: usize = 256;
    let mut chunk = [0u8; CHUNK_PAGES];

    for mapping in query_guard.iter() {
        let range =
            mapping.map_to_addr().max(addr_range.start)..mapping.map_end().min(addr_range.end);
        debug_assert!(range.start.is_multiple_of(PAGE_SIZE));
        debug_assert!(range.end.is_multiple_of(PAGE_SIZE));

        let mut chunk_start = range.start;
        while chunk_start < range.end {
            let chunk_end = (chunk_start + CHUNK_PAGES * PAGE_SIZE).min(range.end);
            let chunk_view = &mut chunk[..(chunk_end - chunk_start) / PAGE_SIZE];
            chunk_view.fill(0);

            mapping.fill_mincore_vec(vmar.vm_space(), chunk_start..chunk_end, chunk_view)?;

            let vec_offset = vec + (chunk_start - addr_range.start) / PAGE_SIZE;
            ctx.user_space().write_slice(vec_offset, chunk_view)?;

            chunk_start = chunk_end;
        }
    }

    Ok(SyscallReturn::Return(0))
}
