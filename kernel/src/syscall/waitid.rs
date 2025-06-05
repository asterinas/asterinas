// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{do_wait, ProcessFilter, WaitOptions},
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
    let process_filter = ProcessFilter::from_which_and_id(which, upid as _, ctx)?;
    let wait_options = WaitOptions::from_bits(options as u32)
        .ok_or(Error::with_message(Errno::EINVAL, "invalid options"))?;

    let wait_status =
        do_wait(process_filter, wait_options, ctx).map_err(|err| match err.error() {
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    let pid = wait_status.map_or(0, |wait_status| wait_status.pid());
    Ok(SyscallReturn::Return(pid as _))
}
