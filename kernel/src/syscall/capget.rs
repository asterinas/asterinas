// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        credentials::c_types::{CUserCapData, CUserCapHeader, LINUX_CAPABILITY_VERSION_3},
        posix_thread::{AsPosixThread, thread_table},
    },
};

pub fn sys_capget(
    cap_user_header_addr: Vaddr,
    cap_user_data_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let cap_user_header = user_space.read_val::<CUserCapHeader>(cap_user_header_addr)?;

    if cap_user_header.version != LINUX_CAPABILITY_VERSION_3 {
        // Write the supported version back to userspace.
        user_space.write_val(cap_user_header_addr, &LINUX_CAPABILITY_VERSION_3)?;
        // If the data pointer is null, return success per Linux behavior.
        if cap_user_data_addr == 0 {
            return Ok(SyscallReturn::Return(0));
        }
        return_errno_with_message!(
            Errno::EINVAL,
            "capability versions other than v3 are not supported"
        );
    };

    if cap_user_data_addr == 0 {
        return Ok(SyscallReturn::Return(0));
    }

    if cap_user_header.pid.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "negative PIDs are not valid");
    }

    let credentials = if cap_user_header.pid != 0 {
        thread_table::get_thread(cap_user_header.pid)
            .ok_or_else(|| Error::with_message(Errno::ESRCH, "the target thread does not exist"))?
            .as_posix_thread()
            .unwrap()
            .credentials()
    } else {
        ctx.posix_thread.credentials()
    };

    let permitted_capset = credentials.permitted_capset();
    let effective_capset = credentials.effective_capset();
    let inheritable_capset = credentials.inheritable_capset();

    // 64-bit capabilities are exposed as two sets of 32-bit fields,
    // so we need to split the capability values up.
    let caps_lo = CUserCapData {
        effective: effective_capset.to_lo_hi().0,
        permitted: permitted_capset.to_lo_hi().0,
        inheritable: inheritable_capset.to_lo_hi().0,
    };
    let caps_hi = CUserCapData {
        effective: effective_capset.to_lo_hi().1,
        permitted: permitted_capset.to_lo_hi().1,
        inheritable: inheritable_capset.to_lo_hi().1,
    };

    user_space.write_val(cap_user_data_addr, &caps_lo)?;
    user_space.write_val(cap_user_data_addr + size_of::<CUserCapData>(), &caps_hi)?;

    Ok(SyscallReturn::Return(0))
}
