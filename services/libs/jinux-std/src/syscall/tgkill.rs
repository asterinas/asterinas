use crate::process::posix_thread::PosixThreadExt;
use crate::thread::{thread_table, Tid};
use crate::{log_syscall_entry, prelude::*};

use crate::process::signal::sig_num::SigNum;
use crate::process::signal::signals::user::{UserSignal, UserSignalKind};
use crate::process::Pid;
use crate::syscall::SYS_TGKILL;

use super::SyscallReturn;

/// tgkill send a signal to a thread with pid as its thread id, and tgid as its thread group id.
/// Since jinuxx only supports one-thread process now, tgkill will send signal to process with pid as its process id,
/// and tgid as its process group id.
pub fn sys_tgkill(tgid: Pid, tid: Tid, sig_num: u8) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_TGKILL);
    let sig_num = SigNum::from_u8(sig_num);
    info!("tgid = {}, pid = {}, sig_num = {:?}", tgid, tid, sig_num);
    let target_thread =
        thread_table::get_thread(tid).ok_or(Error::with_message(Errno::EINVAL, "Invalid pid"))?;
    let posix_thread = target_thread.as_posix_thread().unwrap();
    let pid = posix_thread.process().pid();
    if pid != tgid {
        return_errno_with_message!(
            Errno::EINVAL,
            "the combination of tgid and pid is not valid"
        );
    }
    if target_thread.status().lock().is_exited() {
        return Ok(SyscallReturn::Return(0));
    }
    let signal = {
        let src_pid = current!().pid();
        let src_uid = 0;
        Box::new(UserSignal::new(
            sig_num,
            UserSignalKind::Tkill,
            src_pid,
            src_uid,
        ))
    };
    posix_thread.enqueue_signal(signal);
    Ok(SyscallReturn::Return(0))
}
