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
    let user_space = ctx.user_space();
    let vmar = user_space.vmar();

    // According to linux behavior,
    // <https://elixir.bootlin.com/linux/v6.0.9/source/mm/mprotect.c#L681>,
    // the addr is checked even if len is 0.
    if !addr.is_multiple_of(PAGE_SIZE) {
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

    // On x86_64 and riscv64, `PROT_WRITE` implies `PROT_READ`.
    // Reference:
    // <https://man7.org/linux/man-pages/man2/mprotect.2.html>,
    // Section 5.11.3 from <https://www.intel.com/content/dam/www/public/us/en/documents/manuals/64-ia-32-architectures-software-developer-vol-3a-part-1-manual.pdf>,
    // <https://riscv.github.io/riscv-isa-manual/snapshot/privileged/#translation>.
    #[cfg(any(target_arch = "x86_64", target_arch = "riscv64"))]
    let vm_perms = if !vm_perms.contains(VmPerms::READ) && vm_perms.contains(VmPerms::WRITE) {
        vm_perms | VmPerms::READ
    } else {
        vm_perms
    };

    vmar.protect(vm_perms, range)?;
    Ok(SyscallReturn::Return(0))
}
