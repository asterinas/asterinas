use super::{SyscallReturn, SYS_GETGROUPS};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::credentials;
use crate::util::write_val_to_user;

pub fn sys_getgroups(size: i32, group_list_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETGROUPS);
    debug!("size = {}, group_list_addr = 0x{:x}", size, group_list_addr);

    if size < 0 {
        return_errno_with_message!(Errno::EINVAL, "size cannot be negative");
    }

    let credentials = credentials();
    let groups = credentials.groups();

    if size == 0 {
        return Ok(SyscallReturn::Return(groups.len() as _));
    }

    if groups.len() > size as usize {
        return_errno_with_message!(
            Errno::EINVAL,
            "size is less than the number of supplementary group IDs"
        );
    }

    for (idx, gid) in groups.iter().enumerate() {
        let addr = group_list_addr + idx * core::mem::size_of_val(gid);
        write_val_to_user(addr, gid)?;
    }

    Ok(SyscallReturn::Return(groups.len() as _))
}
