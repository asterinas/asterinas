// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    net::socket::{MessageHeader, SendRecvFlags},
    prelude::*,
    util::{
        net::{get_socket_from_fd, CUserMsgHdr},
        read_val_from_user,
    },
};

pub fn sys_sendmsg(sockfd: FileDesc, user_msghdr_ptr: Vaddr, flags: i32) -> Result<SyscallReturn> {
    let c_user_msghdr: CUserMsgHdr = read_val_from_user(user_msghdr_ptr)?;
    let flags = SendRecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, user_msghdr = {:x?}, flags = {:?}",
        sockfd, c_user_msghdr, flags
    );

    let socket = get_socket_from_fd(sockfd)?;

    let (io_vecs, message_header) = {
        let addr = c_user_msghdr.read_socket_addr_from_user()?;
        let io_vecs = c_user_msghdr.copy_iovs_from_user()?;

        let control_message = {
            if c_user_msghdr.msg_control != 0 {
                // TODO: support sending control message
                warn!("control message is not supported now");
            }
            None
        };

        (io_vecs, MessageHeader::new(addr, control_message))
    };

    let total_bytes = socket.sendmsg(&io_vecs, message_header, flags)?;

    Ok(SyscallReturn::Return(total_bytes as _))
}
