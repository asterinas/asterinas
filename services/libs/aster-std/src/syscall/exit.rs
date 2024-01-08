use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::posix_thread::do_exit;
use crate::process::TermStatus;

use super::{SyscallReturn, SYS_EXIT};

pub fn sys_exit(exit_code: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_EXIT);
    debug!("exid code = {}", exit_code);

    let current_thread = current_thread!();
    let term_status = TermStatus::Exited(exit_code as _);
    do_exit(current_thread, term_status)?;

    Ok(SyscallReturn::Return(0))
}
