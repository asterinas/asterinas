// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::credentials::{
        BOUNDING_CAPSET,
        c_types::{CUserCapData, CUserCapHeader, LINUX_CAPABILITY_VERSION_3},
        capabilities::CapSet,
    },
};

pub fn sys_capset(
    cap_user_header_addr: Vaddr,
    cap_user_data_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let cap_user_header = user_space.read_val::<CUserCapHeader>(cap_user_header_addr)?;

    if cap_user_header.version != LINUX_CAPABILITY_VERSION_3 {
        // Write the supported version back to userspace.
        user_space.write_val(cap_user_header_addr, &LINUX_CAPABILITY_VERSION_3)?;
        return_errno_with_message!(
            Errno::EINVAL,
            "capability versions other than v3 are not supported"
        );
    };

    // The ability to set capabilities of any other thread has been deprecated.
    // Reference: The "With VFS capabilities support" section in
    // <https://man7.org/linux/man-pages/man2/capset.2.html>.
    let header_pid = cap_user_header.pid;
    if header_pid != 0 && header_pid != ctx.posix_thread.tid() {
        return_errno_with_message!(
            Errno::EPERM,
            "setting other threads' capabilities is not allowed"
        );
    }

    let caps_lo = user_space.read_val::<CUserCapData>(cap_user_data_addr)?;
    let caps_hi =
        user_space.read_val::<CUserCapData>(cap_user_data_addr + size_of::<CUserCapData>())?;

    // Convert the 32-bit capability fields to the 64-bit capability value.
    let permitted_capset = CapSet::from_lo_hi(caps_lo.permitted, caps_hi.permitted);
    let effective_capset = CapSet::from_lo_hi(caps_lo.effective, caps_hi.effective);
    let inheritable_capset = CapSet::from_lo_hi(caps_lo.inheritable, caps_hi.inheritable);

    let credentials = ctx.posix_thread.credentials_mut();

    if !credentials.permitted_capset().contains(permitted_capset) {
        return_errno_with_message!(Errno::EPERM, "adding permitted capabilities is not allowed");
    }
    if !permitted_capset.contains(effective_capset) {
        return_errno_with_message!(Errno::EPERM, "effective capabilities are not permitted");
    }
    if !(credentials.inheritable_capset() | credentials.permitted_capset())
        .contains(inheritable_capset)
        && !credentials.effective_capset().contains(CapSet::SETPCAP)
    {
        return_errno_with_message!(Errno::EPERM, "inheritable capabilities are not permitted");
    }
    if !(credentials.inheritable_capset() | BOUNDING_CAPSET).contains(inheritable_capset) {
        return_errno_with_message!(Errno::EPERM, "inheritable capabilities are not bounding");
    }

    credentials.set_permitted_capset(permitted_capset);
    credentials.set_effective_capset(effective_capset);
    credentials.set_inheritable_capset(inheritable_capset);

    Ok(SyscallReturn::Return(0))
}
