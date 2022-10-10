use kxos_frame::debug;

use crate::{
    process::Process,
    syscall::{SyscallResult, SYS_EXIT_GROUP},
};

pub fn sys_exit_group(exit_code: u64) -> SyscallResult {
    debug!("[syscall][id={}][SYS_EXIT_GROUP]", SYS_EXIT_GROUP);
    Process::current().set_exit_code(exit_code as _);
    SyscallResult::Exit(exit_code as _)
}
