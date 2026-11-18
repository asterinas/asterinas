// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{SyscallReturn, mqtimedsend::mq_queue_from_fd};
use crate::{
    prelude::*,
    process::signal::{c_types::{SigNotify, sigevent_t}, sig_num::SigNum},
};

pub(super) fn sys_mqnotify(
    mqdes: i32,
    notification: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let (queue, _) = mq_queue_from_fd(mqdes, ctx)?;

    if notification == 0 {
        // Cancel the notification previously registered by this process.
        queue.unregister_notify(ctx.process.pid())?;
    } else {
        let sig_event: sigevent_t = ctx.user_space().read_val(notification)?;
        let sigev_notify = SigNotify::try_from(sig_event.sigev_notify)?;

        let signal = match sigev_notify {
            SigNotify::SIGEV_NONE => None,
            SigNotify::SIGEV_SIGNAL => {
                let signo = SigNum::try_from(sig_event.sigev_signo as u8)?;
                Some((signo, ctx.process.clone()))
            }
            // TODO: Support `SIGEV_THREAD` and `SIGEV_THREAD_ID`.
            SigNotify::SIGEV_THREAD | SigNotify::SIGEV_THREAD_ID => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the notification method is not supported"
                );
            }
        };

        queue.register_notify(ctx.process.pid(), signal)?;
    }

    Ok(SyscallReturn::Return(0))
}
