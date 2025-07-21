// SPDX-License-Identifier: MPL-2.0

use super::{getrusage::rusage_t, SyscallReturn};
use crate::{
    prelude::*,
    process::{do_wait, ProcessFilter, WaitOptions, WaitStatus},
};

pub fn sys_wait4(
    wait_pid: u64,
    status_ptr: u64,
    wait_options: u32,
    rusage_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let wait_options = WaitOptions::from_bits(wait_options)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown wait option"))?;
    debug!(
        "pid = {}, status_ptr = {}, wait_options: {:?}",
        wait_pid as i32, status_ptr, wait_options
    );
    debug!("wait4 current pid = {}", ctx.process.pid());
    let process_filter = ProcessFilter::from_id(wait_pid as _);

    let wait_status =
        do_wait(process_filter, wait_options, ctx).map_err(|err| match err.error() {
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;
    let Some(wait_status) = wait_status else {
        return Ok(SyscallReturn::Return(0 as _));
    };

    let (return_pid, status_code) = (wait_status.pid(), calculate_status_code(&wait_status));
    if status_ptr != 0 {
        ctx.user_space().write_val(status_ptr as _, &status_code)?;
    }

    if rusage_addr != 0 {
        let rusage = rusage_t {
            ru_utime: wait_status.prof_clock().user_clock().read_time().into(),
            ru_stime: wait_status.prof_clock().kernel_clock().read_time().into(),
            ..Default::default()
        };

        ctx.user_space().write_val(rusage_addr, &rusage)?;
    }

    Ok(SyscallReturn::Return(return_pid as _))
}

fn calculate_status_code(wait_status: &WaitStatus) -> u32 {
    match wait_status {
        WaitStatus::Zombie(process) => process.status().exit_code(),
        WaitStatus::Stop(_, sig_num) => ((sig_num.as_u8() as u32) << 8) | 0x7f,
        WaitStatus::Continue(_) => 0xffff,
    }
}
