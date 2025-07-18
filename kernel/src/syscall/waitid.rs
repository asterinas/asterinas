// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        do_wait,
        signal::{c_types::siginfo_t, constants::SIGCHLD},
        ProcessFilter, WaitOptions,
    },
};

pub fn sys_waitid(
    which: u64,
    upid: u64,
    infoq_addr: u64,
    options: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let process_filter = ProcessFilter::from_which_and_id(which, upid as _)?;
    let wait_options = WaitOptions::from_bits(options as u32)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid options"))?;

    // Check for waitid options
    if !wait_options
        .intersects(WaitOptions::WSTOPPED | WaitOptions::WCONTINUED | WaitOptions::WEXITED)
    {
        return_errno_with_message!(
            Errno::EINVAL,
            "at least one of WSTOPPED, WCONTINUED, or WEXITED should be specified"
        );
    }

    let wait_status =
        do_wait(process_filter, wait_options, ctx).map_err(|err| match err.error() {
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    let Some(wait_status) = wait_status else {
        return Ok(SyscallReturn::Return(0));
    };

    let siginfo = {
        let mut siginfo = siginfo_t::new(SIGCHLD, wait_status.si_code());

        let pid = wait_status.pid();
        let uid = wait_status.uid();
        siginfo.set_pid_uid(pid, uid);

        let status_code = decode_status_code(wait_status.status_code());
        siginfo.set_status(status_code as i32);

        siginfo
    };

    ctx.user_space().write_val(infoq_addr as usize, &siginfo)?;

    Ok(SyscallReturn::Return(0))
}

fn decode_status_code(status_code: u32) -> u32 {
    const KILL_STATUS_MASK: u32 = 0xff;
    // If the status code is a kill status, we return the exit code directly.
    // Otherwise, we shift it right by 8 bits to get the actual exit code.
    if (status_code & KILL_STATUS_MASK) == 0 {
        status_code >> 8
    } else {
        status_code
    }
}
