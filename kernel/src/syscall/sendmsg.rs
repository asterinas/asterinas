// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    net::socket::{MessageHeader, SendRecvFlags},
    prelude::*,
    util::net::{get_socket_from_fd, CUserMsgHdr},
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

    let socket = get_socket_from_fd(sockfd)?;

    let (mut io_vec_reader, message_header) = {
        let addr = c_user_msghdr.read_socket_addr_from_user()?;
        let io_vec_reader = c_user_msghdr.copy_reader_array_from_user(ctx)?;

        let control_message = {
            if c_user_msghdr.msg_control != 0 {
                // TODO: support sending control message
                warn!("control message is not supported now");
            }
            None
        };

        (io_vec_reader, MessageHeader::new(addr, control_message))
    };

    let total_bytes = socket.sendmsg(&mut io_vec_reader, message_header, flags)?;

    Ok(SyscallReturn::Return(total_bytes as _))
}
