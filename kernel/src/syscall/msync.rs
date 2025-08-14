// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, thread::kernel_thread::ThreadOptions};

bitflags! {
    /// Flags for `msync`.
    ///
    /// See <https://elixir.bootlin.com/linux/v6.15.1/source/include/uapi/asm-generic/mman-common.h#L42>.
    pub struct MsyncFlags: i32 {
        /// Performs `msync` asynchronously.
        const MS_ASYNC      = 0x01;
        /// Invalidates cache so that other processes mapping the same file
        /// will immediately see the changes after this `msync` call.
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

    let user_space = ctx.user_space();
    let root_vmar = user_space.root_vmar().dup()?;
    let guard = root_vmar.query(range.clone());
    let mut mappings_iter = guard.iter();

    // Check if the range is fully mapped.
    let Some(first) = mappings_iter.next() else {
        return_errno_with_message!(Errno::ENOMEM, "`msync` called on a not mapped range");
    };
    if first.map_to_addr() > range.start {
        return_partially_mapped!();
    }
    let mut last_end = first.map_end();
    for mapping in mappings_iter {
        let start = mapping.map_to_addr();
        if start != last_end {
            return_partially_mapped!();
        }
        last_end = mapping.map_end();
    }
    if last_end < range.end {
        return_partially_mapped!();
    }

    // Do nothing if not file-backed, as <https://pubs.opengroup.org/onlinepubs/9699919799/> says.
    let inodes = guard
        .iter()
        .filter_map(|m| m.inode().cloned())
        .collect::<Vec<_>>();

    let task_fn = move || {
        for inode in inodes {
            // TODO: Sync a necessary range instead of syncing the whole inode.
            let _ = inode.sync_all();
        }
    };

    // If neither MS_SYNC nor MS_ASYNC is specified, Linux defaults to MS_ASYNC behavior.
    if flags.contains(MsyncFlags::MS_SYNC) {
        task_fn();
    } else {
        ThreadOptions::new(task_fn).spawn();
    }

    Ok(SyscallReturn::Return(0))
}
