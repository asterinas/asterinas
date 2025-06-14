// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    prelude::*,
    vm::vmar::{is_intersected, is_userspace_vaddr, MremapFlags},
};

pub fn sys_mremap(
    old_addr: Vaddr,
    old_size: usize,
    new_size: usize,
    flags: i32,
    new_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = MremapFlags::from_bits(flags).ok_or(Errno::EINVAL)?;
    debug!(
        "mremap: old_addr = 0x{:x}, old_size = {}, new_size = {}, flags = {:?}, new_addr = 0x{:x}",
        old_addr, old_size, new_size, flags, new_addr,
    );

    if old_addr % PAGE_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "mremap: `old_addr` must be page-aligned");
    }
    if new_size == 0 {
        return_errno_with_message!(Errno::EINVAL, "mremap: `new_size` cannot be zero");
    }

    let old_size = old_size.align_up(PAGE_SIZE);
    let old_range = old_addr..old_addr.checked_add(old_size).ok_or(Errno::EINVAL)?;
    let new_size = new_size.align_up(PAGE_SIZE);
    if flags.contains(MremapFlags::MREMAP_FIXED) {
        let new_range = new_addr..new_addr.checked_add(new_size).ok_or(Errno::EINVAL)?;
        if new_addr % PAGE_SIZE != 0
            || !is_userspace_vaddr(new_addr)
            || !is_userspace_vaddr(new_range.end)
        {
            return_errno_with_message!(Errno::EINVAL, "mremap: invalid fixed addr");
        }
        if is_intersected(&old_range, &new_range) {
            return_errno_with_message!(
                Errno::EINVAL,
                "mremap: the new range overlaps with the old one"
            );
        }
        if !flags.contains(MremapFlags::MREMAP_MAYMOVE) {
            return_errno_with_message!(
                Errno::EINVAL,
                "mremap: `MREMAP_FIXED` specified without also specifying `MREMAP_MAYMOVE`"
            );
        }
    }

    // TODO: Add support for `MREMAP_DONTUNMAP` when we need to support `userfaultfd`.
    if flags.contains(MremapFlags::MREMAP_DONTUNMAP) {
        return_errno_with_message!(Errno::EINVAL, "mremap: `MREMAP_DONTUNMAP` is not supported");
    }
    if old_size == 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "mremap: copying shareable mapping is not supported"
        );
    }

    let user_space = ctx.user_space();
    let root_vmar = user_space.root_vmar();
    let new_addr = root_vmar.remap(old_range, new_size, flags, new_addr)?;
    Ok(SyscallReturn::Return(new_addr as _))
}
