// SPDX-License-Identifier: MPL-2.0

use super::{getrusage::rusage_t, SyscallReturn};
use crate::{
    prelude::*,
    process::{wait_child_exit, ProcessFilter, WaitOptions},
};

pub fn sys_wait4(
    wait_pid: u64,
    exit_status_ptr: u64,
    wait_options: u32,
    rusage_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let wait_options = WaitOptions::from_bits(wait_options)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "unknown wait option"))?;
    debug!(
        "pid = {}, exit_status_ptr = {}, wait_options: {:?}",
        wait_pid as i32, exit_status_ptr, wait_options
    );
    debug!("wait4 current pid = {}", ctx.process.pid());
    let process_filter = ProcessFilter::from_id(wait_pid as _);

    let waited_process = wait_child_exit(process_filter, wait_options, ctx)?;
    let Some(process) = waited_process else {
        return Ok(SyscallReturn::Return(0 as _));
    };

    let (return_pid, exit_code) = (process.pid(), process.exit_code());
    if exit_status_ptr != 0 {
        ctx.user_space()
            .write_val(exit_status_ptr as _, &exit_code)?;
    }

    if rusage_addr != 0 {
        let rusage = rusage_t {
            ru_utime: process.prof_clock().user_clock().read_time().into(),
            ru_stime: process.prof_clock().kernel_clock().read_time().into(),
            ..Default::default()
        };

        ctx.user_space().write_val(rusage_addr, &rusage)?;
    }

    Ok(SyscallReturn::Return(return_pid as _))
}
