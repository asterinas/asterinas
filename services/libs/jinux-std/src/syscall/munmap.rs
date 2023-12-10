use crate::log_syscall_entry;
use crate::prelude::*;
use align_ext::AlignExt;
use jinux_frame::early_println;

use super::SyscallReturn;
use super::SYS_MUNMAP;

pub fn sys_munmap(addr: Vaddr, len: usize) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MUNMAP);
    debug!("addr = 0x{:x}, len = {}", addr, len);
    let current = current!();
    let root_vmar = current.root_vmar();
    let len = len.align_up(PAGE_SIZE);
    debug!("unmap range = 0x{:x} - 0x{:x}", addr, addr + len);
    //test for addr overflow
    match addr.checked_add(len) {
        //Some(_)=>early_println!("checked {:?}", addr.checked_add(len)),
        Some(_) => (),
        None => {
            early_println!("overflow!");
            return Err(Error::with_message(Errno::EADDRNOTAVAIL, "Overflow!"));
        }
    }
    root_vmar.destroy(addr..addr + len)?;
    Ok(SyscallReturn::Return(0))
}
