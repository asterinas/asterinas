// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::credentials::c_types::{
        cap_user_data_t, cap_user_header_t, LINUX_CAPABILITY_VERSION_3,
    },
};

pub fn sys_capget(
    cap_user_header_addr: Vaddr,
    cap_user_data_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let cap_user_header: cap_user_header_t =
        user_space.read_val::<cap_user_header_t>(cap_user_header_addr)?;

    if cap_user_header.version != LINUX_CAPABILITY_VERSION_3 {
        return_errno_with_message!(Errno::EINVAL, "not supported (capability version is not 3)");
    };

    // Extract target pid and validate whether it represents the current process.
    let header_pid = cap_user_header.pid;
    // Capget only query current process's credential. Namely, it only allows header->pid == 0
    // or header->pid == getpid(), which are equivalent.
    // See https://linux.die.net/man/2/capget (Section. With VFS capability support) for details.
    if header_pid != 0 && header_pid != ctx.process.pid() {
        return_errno_with_message!(Errno::EINVAL, "invalid pid");
    }

    let credentials = ctx.posix_thread.credentials();
    let inheritable_capset = credentials.inheritable_capset();
    let permitted_capset = credentials.permitted_capset();
    let effective_capset = credentials.effective_capset();

    // Annoying legacy format with 64-bit capabilities exposed as two sets of 32-bit fields,
    // so we need to split the capability values up.
    let result = cap_user_data_t {
        // Note we silently drop the upper capabilities here.
        // This behavior is considered fail-safe behavior.
        effective: effective_capset.as_u32(),
        permitted: permitted_capset.as_u32(),
        inheritable: inheritable_capset.as_u32(),
    };

    user_space.write_val(cap_user_data_addr, &result)?;
    Ok(SyscallReturn::Return(0))
}
