use kxos_frame::vm::VmPerm;

use crate::prelude::*;

use crate::syscall::SYS_MPROTECT;

use super::SyscallResult;

pub fn sys_mprotect(vaddr: u64, len: u64, perms: u64) -> SyscallResult {
    debug!("[syscall][id={}][SYS_MPROTECT]", SYS_MPROTECT);
    let perms = VmPerm::try_from(perms).unwrap();
    do_sys_mprotect(vaddr as Vaddr, len as usize, perms);
    SyscallResult::Return(0)
}

pub fn do_sys_mprotect(addr: Vaddr, len: usize, perms: VmPerm) -> isize {
    debug!("addr = 0x{:x}", addr);
    debug!("len = 0x{:x}", len);
    debug!("perms = {:?}", perms);
    warn!("TODO: mprotect do nothing now");
    0
}
