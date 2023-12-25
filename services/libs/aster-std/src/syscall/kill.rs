use super::{SyscallReturn, SYS_KILL};
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::process::kill;
use crate::process::kill_all;
use crate::process::kill_group;
use crate::process::signal::sig_num::SigNum;
use crate::process::signal::signals::user::{UserSignal, UserSignalKind};
use crate::process::{credentials, ProcessFilter};

pub fn sys_kill(process_filter: u64, sig_num: u64) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_KILL);
    let process_filter = ProcessFilter::from_id(process_filter as _);
    let sig_num = if sig_num == 0 {
        None
    } else {
        Some(SigNum::try_from(sig_num as u8)?)
    };
    debug!(
        "process_filter = {:?}, sig_num = {:?}",
        process_filter, sig_num
    );
    do_sys_kill(process_filter, sig_num)?;
    Ok(SyscallReturn::Return(0))
}

pub fn do_sys_kill(filter: ProcessFilter, sig_num: Option<SigNum>) -> Result<()> {
    let current = current!();

    let signal = sig_num.map(|sig_num| {
        let pid = current.pid();
        let uid = credentials().ruid();
        UserSignal::new(sig_num, UserSignalKind::Kill, pid, uid)
    });

    match filter {
        ProcessFilter::Any => kill_all(signal)?,
        ProcessFilter::WithPid(pid) => kill(pid, signal)?,
        ProcessFilter::WithPgid(pgid) => kill_group(pgid, signal)?,
    }
    Ok(())
}
