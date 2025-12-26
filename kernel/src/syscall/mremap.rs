// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::prelude::*;

pub fn sys_mremap(
    old_addr: Vaddr,
    old_size: usize,
    new_size: usize,
    flags: i32,
    new_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = MremapFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid flags"))?;
    let new_addr = do_sys_mremap(old_addr, old_size, new_size, flags, new_addr, ctx)?;
    Ok(SyscallReturn::Return(new_addr as _))
}

fn do_sys_mremap(
    old_addr: Vaddr,
    old_size: usize,
    new_size: usize,
    flags: MremapFlags,
    new_addr: Vaddr,
    ctx: &Context,
) -> Result<Vaddr> {
    debug!(
        "mremap: old_addr = 0x{:x}, old_size = {}, new_size = {}, flags = {:?}, new_addr = 0x{:x}",
        old_addr, old_size, new_size, flags, new_addr,
    );

    if !old_addr.is_multiple_of(PAGE_SIZE) {
        return_errno_with_message!(Errno::EINVAL, "mremap: `old_addr` must be page-aligned");
    }
    if new_size == 0 {
        return_errno_with_message!(Errno::EINVAL, "mremap: `new_size` cannot be zero");
    }
    if old_size == 0 {
        return_errno_with_message!(
            Errno::EINVAL,
            "mremap: copying shareable mapping is not supported"
        );
    }

    if old_size.checked_add(PAGE_SIZE).is_none() || new_size.checked_add(PAGE_SIZE).is_none() {
        return_errno_with_message!(Errno::EINVAL, "mremap: the size overflows")
    }
    let old_size = old_size.align_up(PAGE_SIZE);
    let new_size = new_size.align_up(PAGE_SIZE);

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();

    if !flags.contains(MremapFlags::MREMAP_FIXED) && new_size <= old_size {
        // We can shrink a old range which spans multiple mappings. See
        // <https://github.com/google/gvisor/blob/95d875276806484f974ce9e95556a561331f8e22/test/syscalls/linux/mremap.cc#L100-L117>.
        vmar.resize_mapping(old_addr, old_size, new_size, false)?;
        return Ok(old_addr);
    }

    if flags.contains(MremapFlags::MREMAP_MAYMOVE) {
        if flags.contains(MremapFlags::MREMAP_FIXED) {
            vmar.remap(old_addr, old_size, Some(new_addr), new_size)
        } else {
            vmar.remap(old_addr, old_size, None, new_size)
        }
    } else {
        if flags.contains(MremapFlags::MREMAP_FIXED) {
            return_errno_with_message!(
                Errno::EINVAL,
                "mremap: `MREMAP_FIXED` specified without also specifying `MREMAP_MAYMOVE`"
            );
        }
        // We can ensure that `new_size > old_size` here. Since we are enlarging
        // the old mapping, it is necessary to check whether the old range lies
        // in a single mapping.
        //
        // FIXME: According to <https://man7.org/linux/man-pages/man2/mremap.2.html>,
        // if the `MREMAP_MAYMOVE` flag is not set, and the mapping cannot
        // be expanded at the current `Vaddr`, we should return an `ENOMEM`.
        // However, `resize_mapping` returns a `EACCES` in this case.
        vmar.resize_mapping(old_addr, old_size, new_size, true)?;
        Ok(old_addr)
    }
}

bitflags! {
    struct MremapFlags: i32 {
        const MREMAP_MAYMOVE = 1 << 0;
        const MREMAP_FIXED = 1 << 1;
        // TODO: Add support for this flag, which exists since Linux 5.7.
        // const MREMAP_DONTUNMAP = 1 << 2;
    }
}
