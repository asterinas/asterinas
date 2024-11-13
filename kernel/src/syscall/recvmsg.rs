// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    net::socket::SendRecvFlags,
    prelude::*,
    util::net::{get_socket_from_fd, CUserMsgHdr},
};

pub fn sys_recvmsg(
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

    let (total_bytes, message_header) = {
        let socket = get_socket_from_fd(sockfd)?;
        let mut io_vec_writer = c_user_msghdr.copy_writer_array_from_user(ctx)?;
        socket.recvmsg(&mut io_vec_writer, flags)?
    };

    if let Some(addr) = message_header.addr() {
        c_user_msghdr.write_socket_addr_to_user(addr)?;
    }

    if c_user_msghdr.msg_control != 0 {
        warn!("receiving control message is not supported");
    }

    Ok(SyscallReturn::Return(total_bytes as _))
}
