// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{wait_child_exit, ProcessFilter, WaitOptions},
};

pub fn sys_waitid(
    which: u64,
    upid: u64,
    _infoq_addr: u64,
    options: u64,
    _rusage_addr: u64,
    ctx: &Context,
) -> Result<SyscallReturn> {
    // FIXME: what does infoq and rusage use for?
    let process_filter = ProcessFilter::from_which_and_id(which, upid)?;
    let wait_options = WaitOptions::from_bits(options as u32)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid options"))?;
    let waited_process = wait_child_exit(process_filter, wait_options, ctx)?;
    let pid = waited_process.map_or(0, |process| process.pid());
    Ok(SyscallReturn::Return(pid as _))
}
