// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    log_syscall_entry,
    prelude::*,
    process::{
        credentials,
        signal::{
            sig_num::SigNum,
            signals::user::{UserSignal, UserSignalKind},
        },
        tgkill, Pid,
    },
    syscall::SYS_TGKILL,
    thread::Tid,
};

/// tgkill send a signal to a thread with pid as its thread id, and tgid as its thread group id.
pub fn sys_tgkill(tgid: Pid, tid: Tid, sig_num: u8) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_TGKILL);
    let sig_num = if sig_num == 0 {
        None
    } else {
        Some(SigNum::try_from(sig_num)?)
    };

    debug!("tgid = {}, pid = {}, sig_num = {:?}", tgid, tid, sig_num);

    let signal = sig_num.map(|sig_num| {
        let pid = current!().pid();
        let uid = credentials().ruid();
        UserSignal::new(sig_num, UserSignalKind::Tkill, pid, uid)
    });
    tgkill(tid, tgid, signal)?;
    Ok(SyscallReturn::Return(0))
}
