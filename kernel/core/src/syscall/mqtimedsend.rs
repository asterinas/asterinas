// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{fs::mqueuefs::MqueueFs, ipc::mqueue::posix, prelude::*};

/// Maximum allowed message priority.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/ipc/mqueue.c#L37>
const MQ_PRIO_MAX: u32 = 32768;

pub(super) fn sys_mqtimedsend(
    mqdes: i32,
    msg_ptr: Vaddr,
    msg_len: usize,
    msg_prio: u32,
    abs_timeout: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    if msg_prio >= MQ_PRIO_MAX {
        return Err(errno!(EINVAL, "the message priority is too high"));
    }

    let (queue, nonblock) = MqueueFs::queue_from_fd(mqdes, ctx)?;

    let attr = queue.attr();
    if msg_len > attr.mq_msgsize as usize {
        return Err(errno!(EMSGSIZE));
    }

    let mut payload = vec![0u8; msg_len].into_boxed_slice();
    ctx.user_space().read_bytes(msg_ptr, &mut payload)?;

    let timeout = posix::parse_timeout(ctx, abs_timeout)?;

    queue
        .send(payload, msg_prio, nonblock, timeout)
        .map_err(|err| match err.error() {
            Errno::EINTR => errno!(ERESTARTSYS),
            _ => err,
        })?;

    Ok(SyscallReturn::Return(0))
}
