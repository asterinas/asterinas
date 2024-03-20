// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    net::socket::SendRecvFlags,
    prelude::*,
    util::{
        net::{get_socket_from_fd, CUserMsgHdr},
        read_val_from_user,
    },
};

pub fn sys_recvmsg(sockfd: FileDesc, user_msghdr_ptr: Vaddr, flags: i32) -> Result<SyscallReturn> {
    let c_user_msghdr: CUserMsgHdr = read_val_from_user(user_msghdr_ptr)?;
    let flags = SendRecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, user_msghdr = {:x?}, flags = {:?}",
        sockfd, c_user_msghdr, flags
    );

    let (total_bytes, message_header) = {
        let socket = get_socket_from_fd(sockfd)?;
        let io_vecs = c_user_msghdr.copy_iovs_from_user()?;
        socket.recvmsg(&io_vecs, flags)?
    };

    if let Some(addr) = message_header.addr() {
        c_user_msghdr.write_socket_addr_to_user(addr)?;
    }

    if c_user_msghdr.msg_control != 0 {
        warn!("receiving control message is not supported");
    }

    Ok(SyscallReturn::Return(total_bytes as _))
}
