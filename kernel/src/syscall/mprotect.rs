// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{prelude::*, vm::perms::VmPerms};

pub fn sys_mprotect(addr: Vaddr, len: usize, perms: u64, ctx: &Context) -> Result<SyscallReturn> {
    let vm_perms = VmPerms::from_bits_truncate(perms as u32);
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}",
        addr, len, vm_perms
    );
    let root_vmar = ctx.process.root_vmar();

    // According to linux behavior,
    // <https://elixir.bootlin.com/linux/v6.0.9/source/mm/mprotect.c#L681>,
    // the addr is checked even if len is 0.
    if addr % PAGE_SIZE != 0 {
        return_errno_with_message!(Errno::EINVAL, "the start address should be page aligned");
    }
    if len == 0 {
        return Ok(SyscallReturn::Return(0));
    }
    if len > isize::MAX as usize {
        return_errno_with_message!(Errno::ENOMEM, "len align overflow");
    }

    let len = len.align_up(PAGE_SIZE);
    let end = addr.checked_add(len).ok_or(Error::with_message(
        Errno::ENOMEM,
        "integer overflow when (addr + len)",
    ))?;
    let range = addr..end;
    root_vmar.protect(vm_perms, range)?;
    Ok(SyscallReturn::Return(0))
}
