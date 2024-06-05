// SPDX-License-Identifier: MPL-2.0

#![allow(unused_variables)]

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{wait_child_exit, ProcessFilter, WaitOptions},
};

pub fn sys_waitid(
    which: u64,
    upid: u64,
    infoq_addr: u64,
    options: u64,
    rusage_addr: u64,
) -> Result<SyscallReturn> {
    // FIXME: what does infoq and rusage use for?
    let process_filter = ProcessFilter::from_which_and_id(which, upid);
    let wait_options = WaitOptions::from_bits(options as u32).expect("Unknown wait options");
    let waited_process = wait_child_exit(process_filter, wait_options)?;
    let pid = waited_process.map_or(0, |process| process.pid());
    Ok(SyscallReturn::Return(pid as _))
}
