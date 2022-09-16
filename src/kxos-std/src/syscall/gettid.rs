use kxos_frame::debug;

use crate::{process::Process, syscall::SYS_GETTID};

use super::SyscallResult;

pub fn sys_gettid() -> SyscallResult {
    debug!("[syscall][id={}][SYS_GETTID]", SYS_GETTID);
    // For single-thread process, tid is equal to pid
    let tid = Process::current().pid();
    SyscallResult::Return(tid as i32)
}
