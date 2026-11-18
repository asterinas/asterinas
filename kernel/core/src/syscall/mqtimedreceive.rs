// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{
    SyscallReturn,
    mqtimedsend::{mq_queue_from_fd, parse_mq_timeout},
};
use crate::prelude::*;

pub(super) fn sys_mqtimedreceive(
    mqdes: i32,
    msg_ptr: Vaddr,
    msg_len: usize,
    msg_prio: Vaddr,
    abs_timeout: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let (queue, nonblock) = mq_queue_from_fd(mqdes, ctx)?;
    let timeout = parse_mq_timeout(ctx, abs_timeout)?;

    let (payload, prio) =
        queue
            .receive(msg_len, nonblock, timeout)
            .map_err(|err| match err.error() {
                Errno::EINTR => Error::new(Errno::ERESTARTSYS),
                _ => err,
            })?;

    ctx.user_space().write_bytes(msg_ptr, &payload)?;
    if msg_prio != 0 {
        ctx.user_space().write_val(msg_prio, &prio)?;
    }

    Ok(SyscallReturn::Return(payload.len() as isize))
}
