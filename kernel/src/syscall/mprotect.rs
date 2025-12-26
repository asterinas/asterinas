// SPDX-License-Identifier: MPL-2.0

use align_ext::AlignExt;

use super::SyscallReturn;
use crate::{
    prelude::*,
    vm::{perms::VmPerms, vmar::VMAR_CAP_ADDR},
};

pub fn sys_mprotect(addr: Vaddr, len: usize, perms: u64, ctx: &Context) -> Result<SyscallReturn> {
    let vm_perms = VmPerms::from_bits_truncate(perms as u32);
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}",
        addr, len, vm_perms
    );

    // According to Linux behavior,
    // <https://elixir.bootlin.com/linux/v6.0.9/source/mm/mprotect.c#L681>,
    // `addr` is checked even if `len` is 0.
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

    let user_space = ctx.user_space();
    let vmar = user_space.vmar();
    vmar.protect(vm_perms, addr_range)?;

    Ok(SyscallReturn::Return(0))
}
