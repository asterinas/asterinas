// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::vmar::VMAR_CAP_ADDR};

pub fn sys_munmap(addr: Vaddr, len: usize, ctx: &Context) -> Result<SyscallReturn> {
    debug!("addr = 0x{:x}, len = {}", addr, len);

    if !addr.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "the mapping address is not aligned");
    }
    if len == 0 {
        return_errno_with_message!(Errno::EINVAL, "the mapping length is zero");
    }
    if VMAR_CAP_ADDR.checked_sub(addr).is_none_or(|gap| gap < len) {
        return_errno_with_message!(Errno::EINVAL, "the mapping range is not in userspace");
    }
    let addr_range = addr..(addr + len).align_up(PAGE_SIZE);

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();
    vmar.remove_mapping(addr_range)?;

    Ok(SyscallReturn::Return(0))
}
