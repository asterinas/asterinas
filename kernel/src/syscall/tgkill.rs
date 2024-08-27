// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        signal::{
            sig_num::SigNum,
            signals::user::{UserSignal, UserSignalKind},
        },
        tgkill, Pid,
    },
    thread::Tid,
};

/// tgkill send a signal to a thread with pid as its thread id, and tgid as its thread group id.
pub fn sys_tgkill(tgid: Pid, tid: Tid, sig_num: u8, ctx: &Context) -> Result<SyscallReturn> {
    let sig_num = if sig_num == 0 {
        None
    } else {
        Some(SigNum::try_from(sig_num)?)
    };

    debug!("tgid = {}, pid = {}, sig_num = {:?}", tgid, tid, sig_num);

    let signal = sig_num.map(|sig_num| {
        let pid = ctx.process.pid();
        let uid = ctx.posix_thread.credentials().ruid();
        UserSignal::new(sig_num, UserSignalKind::Tkill, pid, uid)
    });
    tgkill(tid, tgid, signal, ctx)?;
    Ok(SyscallReturn::Return(0))
}
