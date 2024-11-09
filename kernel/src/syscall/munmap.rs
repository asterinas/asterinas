// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_munmap(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    debug!("addr = 0x{:x}, len = {}", addr, len);

    if addr % PAGE_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "munmap addr must be page-aligned");
    }
    if len == 0 {
        return_errno_with_message!(Errno::EINVAL, "munmap len cannot be zero");
    }
    if len > isize::MAX as usize {
        return_errno_with_message!(Errno::ENOMEM, "munmap len align overflow");
    }

    let root_vmar = ctx.process.root_vmar();
    let len = len.align_up(PAGE_SIZE);
    let end = addr.checked_add(len).ok_or(Error::with_message(
        Errno::EINVAL,
        "integer overflow when (addr + len)",
    ))?;
    debug!("unmap range = 0x{:x} - 0x{:x}", addr, end);
    root_vmar.remove_mapping(addr..end)?;
    Ok(SyscallReturn::Return(0))
}
