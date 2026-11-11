// SPDX-License-Identifier: MPL-2.0

//! Device Mapper ioctl admission and version negotiation.

use ostd::mm::VmIo;

mod abi;
#[cfg(ktest)]
mod tests;

use abi::{DM_IOCTL_MAGIC, decode_command, validate_client_version, write_version};

use crate::{
    context::current_userspace,
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
    util::ioctl::RawIoctl,
};

/// Negotiates the DM ABI before rejecting unsupported commands.
///
/// Linux checks the caller's capability on every ioctl, including version
/// queries. The version exchange touches only three `u32`s, even though the
/// ioctl number encodes a full `dm_ioctl`. A version copyout fault takes
/// precedence over an incompatible client version.
pub(super) fn ioctl(raw_ioctl: RawIoctl) -> Result<i32> {
    if lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        current_thread!().as_posix_thread().unwrap(),
        CapSet::SYS_ADMIN,
    ))
    .is_err()
    {
        return_errno_with_message!(Errno::EACCES, "device-mapper ioctl requires CAP_SYS_ADMIN");
    }

    let raw_cmd = raw_ioctl.cmd();
    if (raw_cmd >> 8) & 0xff != DM_IOCTL_MAGIC {
        return_errno_with_message!(Errno::ENOTTY, "not a device-mapper ioctl");
    }

    // Keep version negotiation ahead of command lookup, as in Linux
    // drivers/md/dm-ioctl.c:check_version/ctl_ioctl (5078a3a23a1e).
    let mut version = [0u8; size_of::<[u32; 3]>()];
    current_userspace!().read_bytes(raw_ioctl.arg(), &mut version)?;
    let version_result = validate_client_version(&version);
    write_version(&mut version)?;
    current_userspace!().write_bytes(raw_ioctl.arg(), &version)?;
    version_result?;

    // The supported command set contains only DM_VERSION.
    decode_command(raw_cmd)?;
    Ok(0)
}
