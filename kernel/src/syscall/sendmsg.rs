// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    net::socket::util::{MessageHeader, SendRecvFlags},
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

    let message_header = {
        let addr = c_user_msghdr.read_socket_addr_from_user()?;
        // Reading control messages may access the file table, so it should be called before
        // `borrow_file_table_mut`.
        let control_messages = c_user_msghdr.read_control_messages_from_user(&user_space)?;
        MessageHeader::new(addr, control_messages)
    };
    let mut io_vec_reader = c_user_msghdr.copy_reader_array_from_user(&user_space)?;

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let total_bytes = socket
        .sendmsg(&mut io_vec_reader, message_header, flags)
        .map_err(|err| match err.error() {
            // FIXME: `sendmsg` should not be restarted if a timeout has been set on the socket using `setsockopt`.
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    Ok(SyscallReturn::Return(total_bytes as _))
}
