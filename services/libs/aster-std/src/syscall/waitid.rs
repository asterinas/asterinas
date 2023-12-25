use crate::process::{wait_child_exit, ProcessFilter};
use crate::{log_syscall_entry, prelude::*};

use crate::process::WaitOptions;

use super::SyscallReturn;
use super::SYS_WAITID;

pub fn sys_waitid(
    which: u64,
    upid: u64,
    infoq_addr: u64,
    options: u64,
    rusage_addr: u64,
) -> Result<SyscallReturn> {
    // FIXME: what does infoq and rusage use for?
    log_syscall_entry!(SYS_WAITID);
    let process_filter = ProcessFilter::from_which_and_id(which, upid);
    let wait_options = WaitOptions::from_bits(options as u32).expect("Unknown wait options");
    let (exit_code, pid) = wait_child_exit(process_filter, wait_options)?;
    Ok(SyscallReturn::Return(pid as _))
}
