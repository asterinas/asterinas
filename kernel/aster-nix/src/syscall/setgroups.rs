// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{credentials_mut, Gid},
};

pub fn sys_setgroups(size: usize, group_list_addr: Vaddr) -> Result<SyscallReturn> {
    debug!("size = {}, group_list_addr = 0x{:x}", size, group_list_addr);

    // TODO: check perm: the calling process should have the CAP_SETGID capability

    if size > NGROUPS_MAX {
        return_errno_with_message!(Errno::EINVAL, "size cannot be greater than NGROUPS_MAX");
    }

    let mut new_groups = BTreeSet::new();
    for idx in 0..size {
        let addr = group_list_addr + idx * core::mem::size_of::<Gid>();
        let gid = CurrentUserSpace::get().read_val(addr)?;
        new_groups.insert(gid);
    }

    let credentials = credentials_mut();
    *credentials.groups_mut() = new_groups;

    Ok(SyscallReturn::Return(0))
}

const NGROUPS_MAX: usize = 65536;
