use crate::prelude::*;

use crate::process::signal::sig_num::SigNum;
use crate::process::signal::signals::user::{UserSignal, UserSignalKind};
use crate::process::{table, Pgid, Pid};
use crate::syscall::SYS_TGKILL;

use super::SyscallReturn;

/// tgkill send a signal to a thread with pid as its thread id, and tgid as its thread group id.
/// Since kxos only supports one-thread process now, tgkill will send signal to process with pid as its process id,
/// and tgid as its process group id.
pub fn sys_tgkill(tgid: Pgid, pid: Pid, sig_num: u8) -> Result<SyscallReturn> {
    debug!("[syscall][id={}][SYS_TGKILL]", SYS_TGKILL);
    let sig_num = SigNum::from_u8(sig_num);
    debug!("tgid = {}, pid = {}, sig_num = {:?}", tgid, pid, sig_num);
    let target_process =
        table::pid_to_process(pid).ok_or(Error::with_message(Errno::EINVAL, "Invalid pid"))?;
    let pgid = target_process.pgid();
    if pgid != tgid {
        return_errno_with_message!(
            Errno::EINVAL,
            "the combination of tgid and pid is not valid"
        );
    }
    if target_process.status().lock().is_zombie() {
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
    let mut sig_queue = target_process.sig_queues().lock();
    sig_queue.enqueue(signal);
    Ok(SyscallReturn::Return(0))
}
