// SPDX-License-Identifier: MPL-2.0

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
    ipc::mqueue::posix::MqAttr,
    prelude::*,
};

pub(super) fn sys_mqgetsetattr(
    mqdes: i32,
    mqstat: Vaddr,
    omqstat: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let new_attr = if mqstat == 0 {
        None
    } else {
        Some(ctx.user_space().read_val::<MqAttr>(mqstat)?)
    };

    // Only `O_NONBLOCK` may be modified via `mq_setattr`.
    if let Some(new_attr) = &new_attr {
        let nonblock = StatusFlags::O_NONBLOCK.bits() as i64;
        if new_attr.mq_flags & !nonblock != 0 {
            return_errno_with_message!(Errno::EINVAL, "only O_NONBLOCK can be modified");
        }
    }

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let fd = FileDesc::try_from(mqdes)?;
    let file = get_file_fast!(&mut file_table, fd);

    // `mq_getsetattr` only applies to message queue file descriptors.
    let inode_handle = file
        .downcast_ref::<InodeHandle>()
        .ok_or_else(|| Error::new(Errno::EBADF))?;
    let queue = inode_handle
        .path()
        .inode()
        .downcast_ref::<MqueueInode>()
        .ok_or_else(|| Error::new(Errno::EBADF))?
        .queue();

    let old_attr = if omqstat == 0 {
        None
    } else {
        // `mq_flags` reflects the `O_NONBLOCK` flag of this file description.
        let mut attr = queue.attr();
        attr.mq_flags = if file.status_flags().contains(StatusFlags::O_NONBLOCK) {
            StatusFlags::O_NONBLOCK.bits() as i64
        } else {
            0
        };
        Some(attr)
    };

    if let Some(new_attr) = new_attr {
        let is_nonblocking = new_attr.mq_flags & (StatusFlags::O_NONBLOCK.bits() as i64) != 0;
        file.update_status_nonblock(is_nonblocking);
    }

    if let Some(old_attr) = old_attr {
        ctx.user_space().write_val(omqstat, &old_attr)?;
    }

    Ok(SyscallReturn::Return(0))
}
