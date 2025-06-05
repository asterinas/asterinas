// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        do_wait,
        signal::{
            c_types::siginfo_t,
            constants::{CLD_CONTINUED, CLD_EXITED, CLD_KILLED, CLD_STOPPED, SIGCHLD, SIGCONT},
        },
        ProcessFilter, WaitOptions, WaitStatus,
    },
};

pub fn sys_waitid(
    which: u64,
    upid: u64,
    infoq_addr: u64,
    options: u64,
    _rusage_addr: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    // FIXME: what does rusage use for?
    let process_filter = ProcessFilter::from_which_and_id(which, upid as _, ctx)?;
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

    if infoq_addr != 0 {
        let siginfo = {
            let (si_code, si_status) = calculate_si_code_and_si_status(&wait_status);
            let pid = wait_status.pid();
            let uid = wait_status.uid();

            let mut siginfo = siginfo_t::new(SIGCHLD, si_code);
            siginfo.set_pid_uid(pid, uid);
            siginfo.set_status(si_status);

            siginfo
        };

        ctx.user_space().write_val(infoq_addr as usize, &siginfo)?;
    }

    Ok(SyscallReturn::Return(0))
}

fn calculate_si_code_and_si_status(wait_status: &WaitStatus) -> (i32, i32) {
    // TODO: Add supports for `CLD_DUMPED` and `CLD_TRAPPED`.
    match wait_status {
        WaitStatus::Zombie(process) => {
            const NORMAL_EXIT_MASK: u32 = 0xff;

            let exit_code = process.status().exit_code();
            // If the process exits normally, the lowest 8 bits of `status_code`
            // will be zero. In this case, we return the actual exit code by
            // shifting the `status_code` right by 8 bits.
            if (exit_code & NORMAL_EXIT_MASK) == 0 {
                (CLD_EXITED, (exit_code >> 8) as i32)
            } else {
                (CLD_KILLED, exit_code as i32)
            }
        }
        WaitStatus::Stop(_process, signum) => (CLD_STOPPED, signum.as_u8() as i32),
        WaitStatus::Continue(_) => (CLD_CONTINUED, SIGCONT.as_u8() as i32),
    }
}
