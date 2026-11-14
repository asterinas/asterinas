// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::Ordering;

use super::{SyscallReturn, get_ioprio::IoPrioWho};
use crate::{
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
};

pub(super) fn sys_ioprio_set(
    which: u32,
    who: u32,
    ioprio: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("which = {}, who = {}, ioprio = {}", which, who, ioprio);

    check_ioprio_class(ioprio, ctx)?;

    let ioprio_who = IoPrioWho::from_which_and_who(which, who, ctx)?;

    match ioprio_who {
        IoPrioWho::Thread(thread) => {
            let target = thread.as_posix_thread().unwrap();
            let target_uid = target.credentials().ruid();
            let caller_credentials = ctx.posix_thread.credentials();

            if target_uid != caller_credentials.euid() && target_uid != caller_credentials.ruid() {
                lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
                    UserNamespace::get_init_singleton().as_ref(),
                    ctx.posix_thread,
                    CapSet::SYS_NICE,
                ))?;
            }

            target.io_priority().store(ioprio, Ordering::Relaxed);

            Ok(SyscallReturn::Return(0))
        }
        IoPrioWho::ProcessGroup(_) => {
            // TODO: Implement process group support
            return_errno!(Errno::EINVAL);
        }
        IoPrioWho::User(_) => {
            // TODO: Implement user support
            return_errno!(Errno::EINVAL);
        }
    }
}

/// Validates the I/O priority class and checks permission for real-time I/O.
fn check_ioprio_class(ioprio: u32, ctx: &Context) -> Result<()> {
    let class = IoPrioClass::try_from((ioprio >> IOPRIO_CLASS_SHIFT) & IOPRIO_CLASS_MASK)?;

    match class {
        IoPrioClass::RealTime => {
            let user_ns = UserNamespace::get_init_singleton();

            if lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
                user_ns.as_ref(),
                ctx.posix_thread,
                CapSet::SYS_ADMIN,
            ))
            .is_err()
            {
                lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
                    user_ns.as_ref(),
                    ctx.posix_thread,
                    CapSet::SYS_NICE,
                ))?;
            }
        }
        IoPrioClass::BestEffort | IoPrioClass::Idle => {}
        IoPrioClass::None => {
            if ioprio & IOPRIO_LEVEL_MASK != 0 {
                return_errno_with_message!(Errno::EINVAL, "invalid I/O priority");
            }
        }
    }

    Ok(())
}

// Linux stores the class in bits 13..=15 and the level in bits 0..=2.
// Bits 3..=12 carry I/O hints and are not part of the level.
const IOPRIO_CLASS_SHIFT: u32 = 13;
const IOPRIO_CLASS_MASK: u32 = 0b111;
const IOPRIO_LEVEL_MASK: u32 = 0b111;

/// Valid I/O priority classes in the Linux ABI.
#[repr(u32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum IoPrioClass {
    None = 0,
    RealTime = 1,
    BestEffort = 2,
    Idle = 3,
}
