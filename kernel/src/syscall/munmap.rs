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

    let root_vmar = ctx.process.root_vmar();
    let len = len.align_up(PAGE_SIZE);
    debug!("unmap range = 0x{:x} - 0x{:x}", addr, addr + len);
    root_vmar.destroy(addr..addr + len)?;
    Ok(SyscallReturn::Return(0))
}
