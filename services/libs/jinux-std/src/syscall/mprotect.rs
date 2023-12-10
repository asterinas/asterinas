use align_ext::AlignExt;
use jinux_frame::early_println;
use crate::{log_syscall_entry, prelude::*};

use crate::syscall::SYS_MPROTECT;
use crate::vm::perms::VmPerms;
use super::SyscallReturn;



pub fn is_align(i: usize) -> Result<()> {
    if i % PAGE_SIZE != 0 {
        return Err(Error::with_message(Errno::EFAULT, "Alignment error")); // 使用字符串作为错误信息
    }
    Ok(())
}

pub fn sys_mprotect(addr: Vaddr, len: usize, perms: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_MPROTECT);
    let vm_perms = VmPerms::from_bits_truncate(perms as u32);
    debug!(
        "addr = 0x{:x}, len = 0x{:x}, perms = {:?}",
        addr, len, vm_perms
    );
    let current = current!();
    let root_vmar = current.root_vmar();
    is_align(addr);
    //debug_assert!(addr % PAGE_SIZE == 0);//这里不应该直接崩掉，修改为报错
    let len = len.align_up(PAGE_SIZE);
    match addr.checked_add(len){
        //Some(_)=>early_println!("checked {:?}", addr.checked_add(len)),
        Some(_)=>(),
        None =>{
            early_println!("overflow!");
            return Err(Error::with_message(Errno::EADDRNOTAVAIL, "Overflow!"));
        }
    }
    //这里会检测出溢出产生panic
    let range = addr..(addr + len);
    root_vmar.protect(vm_perms, range)?;
    Ok(SyscallReturn::Return(0))
}
