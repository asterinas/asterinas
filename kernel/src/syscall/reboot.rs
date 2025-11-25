// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{prelude::*, process::credentials::capabilities::CapSet};

// Linux reboot magic constants.
const LINUX_REBOOT_MAGIC1: i32 = 0xfee1dead_u32 as i32;
const LINUX_REBOOT_MAGIC2: i32 = 0x28121969;
const LINUX_REBOOT_MAGIC2A: i32 = 0x05121996;
const LINUX_REBOOT_MAGIC2B: i32 = 0x16041998;
const LINUX_REBOOT_MAGIC2C: i32 = 0x20112000;

/// Linux reboot commands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[repr(u32)]
enum RebootCmd {
    Restart = 0x01234567,
    Halt = 0xcdef0123,
    PowerOff = 0x4321fedc,
    // TODO: Add more reboot sub-commands.
}

impl TryFrom<i32> for RebootCmd {
    type Error = Error;

    fn try_from(value: i32) -> Result<Self> {
        match value {
            0x01234567 => Ok(Self::Restart),
            0xcdef0123 => Ok(Self::Halt),
            0x4321fedc => Ok(Self::PowerOff),
            _ => return_errno_with_message!(Errno::EINVAL, "invalid reboot command"),
        }
    }
}

pub fn sys_reboot(
    magic1: i32,
    magic2: i32,
    op: i32,
    _arg: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "Reboot syscall invoked with magic1: {:#x}, magic2: {:#x}, op: {:#x}",
        magic1, magic2, op
    );

    // Verify magic numbers
    if magic1 != LINUX_REBOOT_MAGIC1 {
        return_errno_with_message!(Errno::EINVAL, "invalid magic1");
    }

    if magic2 != LINUX_REBOOT_MAGIC2
        && magic2 != LINUX_REBOOT_MAGIC2A
        && magic2 != LINUX_REBOOT_MAGIC2B
        && magic2 != LINUX_REBOOT_MAGIC2C
    {
        return_errno_with_message!(Errno::EINVAL, "invalid magic2");
    }

    if !ctx
        .posix_thread
        .credentials()
        .effective_capset()
        .contains(CapSet::SYS_BOOT)
    {
        return_errno_with_message!(Errno::EPERM, "insufficient capabilities for reboot");
    }

    let cmd = RebootCmd::try_from(op)?;

    match cmd {
        RebootCmd::Restart => {
            // TODO: Implement restart functionality.
            return_errno_with_message!(Errno::ENOSYS, "restart not implemented");
        }
        RebootCmd::Halt | RebootCmd::PowerOff => {
            poweroff();
        }
    }
}

fn poweroff() -> ! {
    use ostd::arch::cpu::poweroff::poweroff;

    // TODO: Perform any necessary cleanup before powering off.
    poweroff()
}
