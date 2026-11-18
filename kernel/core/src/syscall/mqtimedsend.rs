// SPDX-License-Identifier: MPL-2.0

use core::time::Duration;

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::{
        file::{
            InodeHandle, StatusFlags,
            file_table::{FileDesc, get_file_fast},
        },
        mqueuefs::MqueueInode,
    },
    ipc::mqueue::posix::MqueueQueue,
    prelude::*,
    time::{clocks::RealTimeClock, timer::Timeout, timespec_t, wait::ManagedTimeout},
};

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
        return_errno_with_message!(Errno::EINVAL, "the message priority is too high");
    }

    let (queue, nonblock) = mq_queue_from_fd(mqdes, ctx)?;

    let attr = queue.attr();
    if msg_len > attr.mq_msgsize as usize {
        return_errno!(Errno::EMSGSIZE);
    }

    let mut payload = vec![0u8; msg_len].into_boxed_slice();
    ctx.user_space().read_bytes(msg_ptr, &mut payload)?;

    let timeout = parse_mq_timeout(ctx, abs_timeout)?;

    queue
        .send(payload, msg_prio, nonblock, timeout)
        .map_err(|err| match err.error() {
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    Ok(SyscallReturn::Return(0))
}

/// Looks up the queue behind `mqdes` together with its non-blocking flag.
pub(super) fn mq_queue_from_fd(mqdes: i32, ctx: &Context) -> Result<(Arc<MqueueQueue>, bool)> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let fd = FileDesc::try_from(mqdes)?;
    let file = get_file_fast!(&mut file_table, fd);

    // `mq_timedsend` only applies to message queue file descriptors.
    let inode_handle = file
        .downcast_ref::<InodeHandle>()
        .ok_or_else(|| Error::new(Errno::EBADF))?;
    let queue = inode_handle
        .path()
        .inode()
        .downcast_ref::<MqueueInode>()
        .ok_or_else(|| Error::new(Errno::EBADF))?
        .queue()
        .clone();
    let nonblock = file.status_flags().contains(StatusFlags::O_NONBLOCK);

    Ok((queue, nonblock))
}

/// Parses an absolute timeout (`abs_timeout`).
///
/// `abs_timeout` is interpreted against `CLOCK_REALTIME`, matching the POSIX
/// semantics of `mq_timedsend` / `mq_timedreceive`.
pub(super) fn parse_mq_timeout(
    ctx: &Context,
    abs_timeout: Vaddr,
) -> Result<Option<ManagedTimeout<'static>>> {
    if abs_timeout == 0 {
        return Ok(None);
    }

    let time_spec: timespec_t = ctx.user_space().read_val(abs_timeout)?;
    let duration = Duration::try_from(time_spec)?;

    Ok(Some(ManagedTimeout::new_with_manager(
        Timeout::When(duration),
        RealTimeClock::timer_manager(),
    )))
}
