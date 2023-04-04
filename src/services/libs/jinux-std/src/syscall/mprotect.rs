use align_ext::AlignExt;

use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_MPROTECT;
use crate::vm::perms::VmPerms;

use super::SyscallReturn;

pub fn sys_mprotect(addr: Vaddr, len: usize, perms: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MPROTECT);
    let vm_perms = VmPerms::from_bits_truncate(perms as u32);
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}",
        addr, len, vm_perms
    );
    let current = current!();
    let root_vmar = current.root_vmar();
    debug_assert!(addr % PAGE_SIZE == 0);
    let len = len.align_up(PAGE_SIZE);
    let range = addr..(addr + len);
    root_vmar.protect(vm_perms, range)?;
    Ok(SyscallReturn::Return(0))
}
