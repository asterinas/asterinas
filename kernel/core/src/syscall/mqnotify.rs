// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{SyscallReturn, mqtimedsend::mq_queue_from_fd};
use crate::{
    prelude::*,
    process::signal::{
        c_types::{SigNotify, sigevent_t},
        sig_num::SigNum,
    },
};

pub(super) fn sys_mqnotify(
    mqdes: i32,
    notification: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if notification == 0 {
        // Cancel the notification previously registered by this process. An
        // attempt to cancel a notification registered by another process is
        // silently ignored, matching Linux.
        let (queue, _) = mq_queue_from_fd(mqdes, ctx)?;
        queue.unregister_notify(ctx.process.pid());
    } else {
        // Parse and validate the notification before resolving the descriptor,
        // so that invalid arguments report `EINVAL` rather than `EBADF`.
        let sig_event: sigevent_t = ctx.user_space().read_val(notification)?;
        let sigev_notify = SigNotify::try_from(sig_event.sigev_notify)?;

        let signal = match sigev_notify {
            SigNotify::SIGEV_NONE => None,
            SigNotify::SIGEV_SIGNAL => {
                // Convert through `u8` explicitly so that out-of-range values
                // (negative or larger than 255) cannot wrap into a valid
                // signal number.
                let Ok(signo) = u8::try_from(sig_event.sigev_signo) else {
                    return_errno_with_message!(Errno::EINVAL, "invalid signal number");
                };
                let signo = SigNum::try_from(signo)?;
                Some((signo, ctx.process.clone(), sig_event.sigev_value))
            }
            // TODO: Support `SIGEV_THREAD` and `SIGEV_THREAD_ID`.
            SigNotify::SIGEV_THREAD | SigNotify::SIGEV_THREAD_ID => {
                return_errno_with_message!(
                    Errno::EINVAL,
                    "the notification method is not supported"
                );
            }
        };

        let (queue, _) = mq_queue_from_fd(mqdes, ctx)?;
        queue.register_notify(ctx.process.pid(), signal)?;
    }

    Ok(SyscallReturn::Return(0))
}
