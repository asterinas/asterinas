// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    net::socket::SendRecvFlags,
    prelude::*,
    util::net::CUserMsgHdr,
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

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let (total_bytes, message_header) = {
        let user_space = ctx.user_space();
        let mut io_vec_writer = c_user_msghdr.copy_writer_array_from_user(&user_space)?;
        socket
            .recvmsg(&mut io_vec_writer, flags)
            .map_err(|err| match err.error() {
                // FIXME: `recvmsg` should not be restarted if a timeout has been set on the socket using `setsockopt`.
                Errno::EINTR => Error::new(Errno::ERESTARTSYS),
                _ => err,
            })?
    };

    if let Some(addr) = message_header.addr() {
        c_user_msghdr.write_socket_addr_to_user(addr)?;
    }

    if c_user_msghdr.msg_control != 0 {
        warn!("receiving control message is not supported");
    }

    Ok(SyscallReturn::Return(total_bytes as _))
}
