use crate::{log_syscall_entry, prelude::*};

use crate::process::process_table;
use crate::process::signal::signals::user::{UserSignal, UserSignalKind};
use crate::{
    process::{signal::sig_num::SigNum, ProcessFilter},
    syscall::SYS_KILL,
};

use super::SyscallReturn;

pub fn sys_kill(process_filter: u64, sig_num: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_KILL);
    let process_filter = ProcessFilter::from_id(process_filter as _);
    let sig_num = SigNum::try_from(sig_num as u8).unwrap();
    debug!(
        "process_filter = {:?}, sig_num = {:?}",
        process_filter, sig_num
    );
    do_sys_kill(process_filter, sig_num)?;
    Ok(SyscallReturn::Return(0))
}

pub fn do_sys_kill(filter: ProcessFilter, sig_num: SigNum) -> Result<()> {
    let current = current!();
    let pid = current.pid();
    // FIXME: use the correct uid
    let uid = 0;
    let signal = UserSignal::new(sig_num, UserSignalKind::Kill, pid, uid);
    match filter {
        ProcessFilter::Any => {
            for process in process_table::get_all_processes() {
                process.enqueue_signal(Box::new(signal));
            }
        }
        ProcessFilter::WithPid(pid) => {
            if let Some(process) = process_table::get_process(&pid) {
                process.enqueue_signal(Box::new(signal));
            } else {
                return_errno_with_message!(Errno::ESRCH, "No such process in process table");
            }
        }
        ProcessFilter::WithPgid(pgid) => {
            if let Some(process_group) = process_table::get_process_group(&pgid) {
                process_group.broadcast_signal(signal);
            } else {
                return_errno_with_message!(Errno::ESRCH, "No such process group in process table");
            }
        }
    }
    Ok(())
}
