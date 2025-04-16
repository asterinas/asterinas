// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{CloneFlags, PidNamespace},
};

pub fn sys_unshare(unshare_flags: u32, ctx: &Context) -> Result<SyscallReturn> {
    let unshare_flags = CloneFlags::from_bits_truncate(unshare_flags);

    if unshare_flags.contains(CloneFlags::CLONE_NEWPID) {
        let pid_ns_for_children = ctx.process.pid_ns_for_children();

        if pid_ns_for_children.get().is_some() {
            return_errno_with_message!(
                Errno::EINVAL,
                "pid_ns_for_children cannot be initialized twice"
            );
        }

        let new_pid_ns = PidNamespace::new_child(ctx.process.pid_namespace())?;
        pid_ns_for_children.call_once(|| new_pid_ns);
    }

    Ok(SyscallReturn::Return(0))
}
