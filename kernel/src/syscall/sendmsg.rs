// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    net::socket::{MessageHeader, SendRecvFlags},
    prelude::*,
    util::net::CUserMsgHdr,
};

pub fn sys_sendmsg(
    sockfd: FileDesc,
    user_msghdr_ptr: Vaddr,
    flags: i32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let c_user_msghdr: CUserMsgHdr = ctx.user_space().read_val(user_msghdr_ptr)?;
    let flags = SendRecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, user_msghdr = {:x?}, flags = {:?}",
        sockfd, c_user_msghdr, flags
    );

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let user_space = ctx.user_space();
    let (mut io_vec_reader, message_header) = {
        let addr = c_user_msghdr.read_socket_addr_from_user()?;
        let io_vec_reader = c_user_msghdr.copy_reader_array_from_user(&user_space)?;

        let control_message = {
            if c_user_msghdr.msg_control != 0 {
                // TODO: support sending control message
                warn!("control message is not supported now");
            }
            None
        };

        (io_vec_reader, MessageHeader::new(addr, control_message))
    };

    let total_bytes = socket
        .sendmsg(&mut io_vec_reader, message_header, flags)
        .map_err(|err| match err.error() {
            // FIXME: `sendmsg` should not be restarted if a timeout has been set on the socket using `setsockopt`.
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    Ok(SyscallReturn::Return(total_bytes as _))
}
