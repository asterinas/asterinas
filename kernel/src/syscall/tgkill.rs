// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    prelude::*,
    process::{
        Pid,
        signal::{
            sig_num::SigNum,
            signals::{
                Signal,
                user::{UserSignal, UserSignalKind},
            },
        },
        tgkill,
    },
    thread::Tid,
};

/// Sends a signal to a thread with `tid` as its thread ID, and `tgid` as its thread group ID.
pub fn sys_tgkill(tgid: Pid, tid: Tid, sig_num: u8, ctx: &Context) -> Result<SyscallReturn> {
    let sig_num = if sig_num == 0 {
        None
    } else {
        Some(SigNum::try_from(sig_num)?)
    };
    debug!("tgid = {}, pid = {}, sig_num = {:?}", tgid, tid, sig_num);

    if tgid.cast_signed() < 0 || tid.cast_signed() < 0 {
        return_errno_with_message!(Errno::EINVAL, "negative TGIDs or TIDs are not valid");
    }

    let signal = sig_num.map(|sig_num| {
        let pid = ctx.process.pid();
        let uid = ctx.posix_thread.credentials().ruid();
        Box::new(UserSignal::new(sig_num, UserSignalKind::Tkill, pid, uid)) as Box<dyn Signal>
    });
    tgkill(tid, tgid, signal, ctx)?;
    Ok(SyscallReturn::Return(0))
}
