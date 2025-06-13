// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    net::socket::util::{MessageHeader, SendRecvFlags},
    prelude::*,
    util::net::read_socket_addr_from_user,
};

pub fn sys_sendto(
    sockfd: FileDesc,
    buf: Vaddr,
    len: usize,
    flags: i32,
    dest_addr: Vaddr,
    addrlen: usize,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = SendRecvFlags::from_bits_truncate(flags);
    let socket_addr = if dest_addr == 0 {
        None
    } else {
        let socket_addr = read_socket_addr_from_user(dest_addr, addrlen)?;
        Some(socket_addr)
    };
    debug!("sockfd = {sockfd}, buf = 0x{buf:x}, len = 0x{len:x}, flags = {flags:?}, socket_addr = {socket_addr:?}");

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let message_header = MessageHeader::new(socket_addr, Vec::new());

    let user_space = ctx.user_space();
    let mut reader = user_space.reader(buf, len)?;
    let send_size = socket
        .sendmsg(&mut reader, message_header, flags)
        .map_err(|err| match err.error() {
            // FIXME: `sendto` should not be restarted if a timeout has been set on the socket using `setsockopt`.
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    Ok(SyscallReturn::Return(send_size as _))
}
