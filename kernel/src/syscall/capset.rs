// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::credentials::{
        c_types::{cap_user_data_t, cap_user_header_t, LINUX_CAPABILITY_VERSION_3},
        capabilities::CapSet,
    },
};

fn make_kernel_cap(low: u32, high: u32) -> u64 {
    ((low as u64) | ((high as u64) << 32)) & ((1u64 << (CapSet::most_significant_bit() + 1)) - 1)
}

pub fn sys_capset(
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

    // The ability to set capabilities of any other process has been deprecated.
    // See: https://elixir.bootlin.com/linux/v6.9.3/source/kernel/capability.c#L209 for more details.
    let header_pid = cap_user_header.pid;
    if header_pid != 0 && header_pid != ctx.process.pid() {
        return_errno_with_message!(Errno::EINVAL, "invalid pid");
    }

    // Convert the cap(u32) to u64
    let cap_user_data: cap_user_data_t =
        user_space.read_val::<cap_user_data_t>(cap_user_data_addr)?;
    let inheritable = make_kernel_cap(cap_user_data.inheritable, 0);
    let permitted = make_kernel_cap(cap_user_data.permitted, 0);
    let effective = make_kernel_cap(cap_user_data.effective, 0);

    let credentials = ctx.posix_thread.credentials_mut();

    credentials.set_inheritable_capset(CapSet::from_bits_truncate(inheritable));
    credentials.set_permitted_capset(CapSet::from_bits_truncate(permitted));
    credentials.set_effective_capset(CapSet::from_bits_truncate(effective));

    Ok(SyscallReturn::Return(0))
}
