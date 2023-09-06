use super::SyscallReturn;
use super::SYS_SET_TID_ADDRESS;
use crate::process::posix_thread::PosixThreadExt;
use crate::{log_syscall_entry, prelude::*};

pub fn sys_set_tid_address(tidptr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SET_TID_ADDRESS);
    debug!("tidptr = 0x{:x}", tidptr);
    let current_thread = current_thread!();
    let posix_thread = current_thread.as_posix_thread().unwrap();
    let mut clear_child_tid = posix_thread.clear_child_tid().lock();
    if *clear_child_tid != 0 {
        // According to manuals at https://man7.org/linux/man-pages/man2/set_tid_address.2.html
        // We need to write 0 to clear_child_tid and do futex wake
        todo!()
    } else {
        *clear_child_tid = tidptr;
    }
    let tid = current_thread.tid();
    Ok(SyscallReturn::Return(tid as _))
}
