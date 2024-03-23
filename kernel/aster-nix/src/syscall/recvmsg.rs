// SPDX-License-Identifier: MPL-2.0

use aster_frame::vm::Vaddr;

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDescripter,
    log_syscall_entry,
    net::socket::{MessageHeader, SendRecvFlags},
    prelude::*,
    syscall::SYS_RECVMSG,
    util::{
        net::{get_socket_from_fd, write_socket_addr_with_max_len, CUserMsgHdr},
        read_val_from_user, IoVecIter,
    },
};

pub fn sys_recvmsg(
    sockfd: FileDescripter,
    user_msghdr_ptr: Vaddr,
    flags: i32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RECVMSG);

    let c_user_msghdr: CUserMsgHdr = read_val_from_user(user_msghdr_ptr)?;
    let flags = SendRecvFlags::from_bits_truncate(flags);

    debug!(
        "sockfd = {}, user_msghdr = {:x?}, flags = {:?}",
        sockfd, c_user_msghdr, flags
    );

    let socket = get_socket_from_fd(sockfd)?;

    let mut message_header = {
        let io_vec_iter = IoVecIter::new(c_user_msghdr.msg_iov, c_user_msghdr.msg_iovlen as usize);
        MessageHeader::new(None, io_vec_iter, None)
    };

    let total_bytes = socket.recvmsg(&mut message_header, flags)?;

    if c_user_msghdr.msg_name != 0 {
        debug_assert!(message_header.addr().is_some());
        write_socket_addr_with_max_len(
            message_header.addr().unwrap(),
            c_user_msghdr.msg_name,
            c_user_msghdr.msg_namelen,
        )?;
    }

    if c_user_msghdr.msg_control != 0 {
        warn!("Receiving control message is not supported")
    }

    Ok(SyscallReturn::Return(total_bytes as _))
}
