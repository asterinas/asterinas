use crate::process::{process_filter::ProcessFilter, wait::wait_child_exit};

use super::SyscallResult;
use crate::process::wait::WaitOptions;

pub fn sys_waitid(
    which: u64,
    upid: u64,
    infoq_addr: u64,
    options: u64,
    rusage_addr: u64,
) -> SyscallResult {
    // FIXME: what does infoq and rusage use for?
    let process_filter = ProcessFilter::from_which_and_id(which, upid);
    let wait_options = WaitOptions::from_bits(options as u32).expect("Unknown wait options");
    let (exit_code, pid) = wait_child_exit(process_filter, wait_options);
    SyscallResult::Return(pid)
}
