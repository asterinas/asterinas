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
        DFLT_MSGMAX, DFLT_MSGSIZEMAX, HARD_MSGMAX, HARD_MSGSIZEMAX, MqAttr, create_queue,
        lookup_queue,
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

    let queue = if creation_flags.contains(CreationFlags::O_CREAT) {
        let attr = read_attr(ctx, attr_addr)?;
        validate_attr(attr)?;

        match create_queue(&name, InodeMode::from_bits_truncate(mode), attr) {
            Ok(queue) => queue,
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
    let fd = file_table.unwrap().lock_and_insert(file_handle, fd_flags)?;

    Ok(SyscallReturn::Return(fd.into()))
}

/// Reads `struct mq_attr` from user space.
///
/// A null `attr` pointer means that the queue should be created with the
/// default attributes.
fn read_attr(ctx: &Context, attr_addr: Vaddr) -> Result<MqAttr> {
    if attr_addr == 0 {
        return Ok(MqAttr {
            mq_maxmsg: DFLT_MSGMAX,
            mq_msgsize: DFLT_MSGSIZEMAX,
            ..MqAttr::default()
        });
    }
    Ok(ctx.user_space().read_val::<MqAttr>(attr_addr)?)
}

/// Validates the attributes passed to `mq_open` with `O_CREAT`.
fn validate_attr(attr: MqAttr) -> Result<()> {
    if attr.mq_maxmsg <= 0 || attr.mq_msgsize <= 0 {
        return_errno_with_message!(Errno::EINVAL, "mq_maxmsg and mq_msgsize must be positive");
    }
    if attr.mq_maxmsg > HARD_MSGMAX || attr.mq_msgsize > HARD_MSGSIZEMAX {
        return_errno_with_message!(
            Errno::EINVAL,
            "mq_maxmsg or mq_msgsize exceeds the hard limit"
        );
    }
    Ok(())
}
