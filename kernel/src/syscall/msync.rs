// SPDX-License-Identifier: MPL-2.0

#![expect(unused)]

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::prelude::*;

bitflags! {
    /// Flags for `msync`.
    ///
    /// See <https://elixir.bootlin.com/linux/v6.15.1/source/include/uapi/asm-generic/mman-common.h#L42>.
    pub struct MsyncFlags: i32 {
        /// Performs `msync` asynchronously.
        const MS_ASYNC      = 0x01;
        /// Invalidates cache so that other processes mapping the same file
        /// will immediately see the changes before this `msync` call.
        ///
        /// Should be a no-op since we use the same page cache for all processes.
        const MS_INVALIDATE = 0x02;
        /// Performs `msync` synchronously.
        const MS_SYNC       = 0x04;
    }
}

macro_rules! return_partially_mapped {
    () => {
        return_errno_with_message!(Errno::ENOMEM, "`msync` called on a partially mapped range")
    };
}

pub fn sys_msync(start: Vaddr, size: usize, flag: i32, ctx: &Context) -> Result<SyscallReturn> {
    let flags = MsyncFlags::from_bits(flag).ok_or_else(|| Error::new(Errno::EINVAL))?;

    debug!("msync: start = {start:#x}, size = {size}, flags = {flags:?}");

    if start % PAGE_SIZE != 0 || flags.contains(MsyncFlags::MS_ASYNC | MsyncFlags::MS_SYNC) {
        return_errno!(Errno::EINVAL);
    }

    if size == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    let range = {
        let end = start
            .checked_add(size)
            .ok_or(Error::with_message(
                Errno::EINVAL,
                "`msync` `size` overflows",
            ))?
            .align_up(PAGE_SIZE);
        start..end
    };

    // TODO

    Ok(SyscallReturn::Return(0))
}
