use crate::{
    memory::write_val_to_user,
    process::{process_filter::ProcessFilter, wait::wait_child_exit},
    syscall::SYS_WAIT4,
};

use super::SyscallResult;
use crate::prelude::*;
use crate::process::wait::WaitOptions;

pub fn sys_wait4(wait_pid: u64, exit_status_ptr: u64, wait_options: u64) -> SyscallResult {
    debug!("[syscall][id={}][SYS_WAIT4]", SYS_WAIT4);
    let wait_options = WaitOptions::from_bits(wait_options as u32).expect("Unknown wait options");
    debug!("pid = {}", wait_pid as isize);
    debug!("exit_status_ptr = {}", exit_status_ptr);
    debug!("wait_options: {:?}", wait_options);
    let process_filter = ProcessFilter::from_id(wait_pid as _);
    let (return_pid, exit_code) = wait_child_exit(process_filter, wait_options);
    if return_pid != 0 && exit_status_ptr != 0 {
        write_val_to_user(exit_status_ptr as _, &exit_code);
    }

    SyscallResult::Return(return_pid as _)
}
