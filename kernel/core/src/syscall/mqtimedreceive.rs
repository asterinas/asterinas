// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{fs::mqueuefs::MqueueFs, ipc::mqueue::posix, prelude::*};

pub(super) fn sys_mqtimedreceive(
    mqdes: i32,
    msg_ptr: Vaddr,
    msg_len: usize,
    msg_prio: Vaddr,
    abs_timeout: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let (queue, nonblock) = MqueueFs::queue_from_fd(mqdes, ctx)?;
    let timeout = posix::parse_timeout(ctx, abs_timeout)?;

    let (payload, prio) =
        queue
            .receive(msg_len, nonblock, timeout)
            .map_err(|err| match err.error() {
                Errno::EINTR => errno!(ERESTARTSYS),
                _ => err,
            })?;

    ctx.user_space().write_bytes(msg_ptr, &payload)?;
    if msg_prio != 0 {
        ctx.user_space().write_val(msg_prio, &prio)?;
    }

    Ok(SyscallReturn::Return(payload.len() as isize))
}
