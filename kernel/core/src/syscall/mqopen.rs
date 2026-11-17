// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::{SyscallReturn, constants::MAX_FILENAME_LEN};
use crate::{
    fs::{
        file::{
            AccessMode, CreationFlags, InodeMode, StatusFlags,
            file_table::{FdFlags, FileTableInsert},
        },
        mqueuefs::MqueueFs,
    },
    ipc::mqueue::posix::{
        MqAttr, create_queue, lookup_queue, msg_default, msg_max, msgsize_default, msgsize_max,
        unlink_queue,
    },
    prelude::*,
};

pub(super) fn sys_mqopen(
    name_addr: Vaddr,
    oflag: i32,
    mode: u16,
    attr_addr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    // glibc and musl can delete '/' at the beginning of the name string.
    // Name validation (empty names and '/') is delegated to `mqueue::posix`.
    let name = ctx.user_space().read_cstring(name_addr, MAX_FILENAME_LEN)?;
    let name = name.to_string_lossy();

    let creation_flags = CreationFlags::from_bits_truncate(oflag as u32);
    let access_mode = AccessMode::from_u32(oflag as u32)?;
    let status_flags = StatusFlags::from_bits_truncate(oflag as u32);

    let mqueue = MqueueFs::singleton();

    let mut created = false;
    let queue = if creation_flags.contains(CreationFlags::O_CREAT) {
        let attr = read_attr(ctx, attr_addr)?;

        match create_queue(&name, InodeMode::from_bits_truncate(mode), attr) {
            Ok(queue) => {
                created = true;
                queue
            }
            Err(err) if err.error() == Errno::EEXIST => {
                if creation_flags.contains(CreationFlags::O_EXCL) {
                    return Err(err);
                }
                lookup_queue(&name)?
            }
            Err(err) => return Err(err),
        }
    } else {
        lookup_queue(&name)?
    };

    let file_handle = mqueue.open_queue(queue, access_mode, status_flags)?;

    let fd_flags = if creation_flags.contains(CreationFlags::O_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    let file_table = ctx.thread_local.borrow_file_table();
    let fd = match file_table.unwrap().lock_and_insert(file_handle, fd_flags) {
        Ok(fd) => fd,
        Err(err) => {
            // Undo the queue creation so that a failed `mq_open` (e.g., with
            // `EMFILE`) does not leave the queue behind. Linux achieves the
            // same by allocating the fd before creating the queue.
            if created {
                let _ = unlink_queue(&name);
            }
            return Err(err);
        }
    };

    Ok(SyscallReturn::Return(fd.into()))
}

/// Reads `struct mq_attr` from user space.
///
/// A null `attr` pointer means that the queue should be created with the
/// default attributes, which are bounded by the mqueue sysctls.
fn read_attr(ctx: &Context, attr_addr: Vaddr) -> Result<MqAttr> {
    if attr_addr == 0 {
        return Ok(MqAttr {
            mq_maxmsg: msg_default().min(msg_max()),
            mq_msgsize: msgsize_default().min(msgsize_max()),
            ..MqAttr::default()
        });
    }
    Ok(ctx.user_space().read_val::<MqAttr>(attr_addr)?)
}
