// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SETGROUPS};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::{credentials_mut, Gid};
use crate::util::read_val_from_user;

pub fn sys_setgroups(size: usize, group_list_addr: Vaddr) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SETGROUPS);
    debug!("size = {}, group_list_addr = 0x{:x}", size, group_list_addr);

    // TODO: check perm: the calling process should have the CAP_SETGID capability

    if size > NGROUPS_MAX {
        return_errno_with_message!(Errno::EINVAL, "size cannot be greater than NGROUPS_MAX");
    }

    let mut new_groups = BTreeSet::new();
    for idx in 0..size {
        let addr = group_list_addr + idx * core::mem::size_of::<Gid>();
        let gid = read_val_from_user(addr)?;
        new_groups.insert(gid);
    }

    let credentials = credentials_mut();
    *credentials.groups_mut() = new_groups;

    Ok(SyscallReturn::Return(0))
}

const NGROUPS_MAX: usize = 65536;
