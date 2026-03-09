// SPDX-License-Identifier: MPL-2.0

use ostd::mm::VmIo;

use super::SyscallReturn;
use crate::{
    fs::file::file_table::FileDesc,
    net::socket::{
        Socket,
        util::{MessageHeader, SendRecvFlags},
    },
    prelude::*,
    util::net::CUserMsgHdr,
};

pub fn sys_sendmsg(
    sockfd: FileDesc,
    user_msghdr_ptr: Vaddr,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let user_space = ctx.user_space();
    let c_user_msghdr: CUserMsgHdr = user_space.read_val(user_msghdr_ptr)?;
    let flags = SendRecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, user_msghdr = {:x?}, flags = {:?}",
        sockfd, c_user_msghdr, flags
    );

    let file = {
        // Reading control messages may access the file table,
        // so we have to clone the file and drop the file table reference here.
        let file_table = ctx.thread_local.borrow_file_table();
        let file_table_locked = file_table.unwrap().read();
        file_table_locked.get_file(sockfd)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    let total_bytes = send_one_message(socket, &c_user_msghdr, &user_space, flags)?;

    Ok(SyscallReturn::Return(total_bytes as _))
}

pub(super) fn send_one_message(
    socket: &dyn Socket,
    c_user_msghdr: &CUserMsgHdr,
    user_space: &CurrentUserSpace,
    flags: SendRecvFlags,
) -> Result<usize> {
    let message_header = {
        let addr = c_user_msghdr.read_socket_addr_from_user()?;
        let control_messages = c_user_msghdr.read_control_messages_from_user(user_space)?;
        MessageHeader::new(addr, control_messages)
    };
    let mut io_vec_reader = c_user_msghdr.copy_reader_array_from_user(user_space)?;

    socket
        .sendmsg(&mut io_vec_reader, message_header, flags)
        .map_err(|err| match err.error() {
            // FIXME: `sendmsg` should not be restarted if a timeout has been set on the socket using `setsockopt`.
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })
}
