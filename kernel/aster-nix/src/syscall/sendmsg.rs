// SPDX-License-Identifier: MPL-2.0

use aster_frame::vm::Vaddr;

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDescripter,
    log_syscall_entry,
    net::socket::{MessageHeader, SendRecvFlags},
    prelude::*,
    syscall::SYS_SENDMSG,
    util::{
        net::{get_socket_from_fd, CUserMsgHdr},
        read_val_from_user, IoVecIter,
    },
};

pub fn sys_sendmsg(
    sockfd: FileDescripter,
    user_msghdr_ptr: Vaddr,
    flags: i32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SENDMSG);

    let c_user_msghdr: CUserMsgHdr = read_val_from_user(user_msghdr_ptr)?;
    let flags = SendRecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, user_msghdr = {:x?}, flags = {:?}",
        sockfd, c_user_msghdr, flags
    );

    let socket = get_socket_from_fd(sockfd)?;

    let message_header = {
        let addr = c_user_msghdr.read_socket_addr()?;

        let io_vec_iter = IoVecIter::new(c_user_msghdr.msg_iov, c_user_msghdr.msg_iovlen as usize);

        let control_message = {
            if c_user_msghdr.msg_control != 0 {
                // TODO: support sending control message
                warn!("control message is not supported now");
            }
            None
        };

        MessageHeader::new(addr, io_vec_iter, control_message)
    };

    let total_bytes = socket.sendmsg(message_header, flags)?;

    Ok(SyscallReturn::Return(total_bytes as _))
}
