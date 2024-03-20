// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    net::socket::{MessageHeader, SendRecvFlags},
    prelude::*,
    util::{
        net::{get_socket_from_fd, read_socket_addr_from_user},
        IoVec,
    },
};

pub fn sys_sendto(
    sockfd: FileDesc,
    buf: Vaddr,
    len: usize,
    flags: i32,
    dest_addr: Vaddr,
    addrlen: usize,
) -> Result<SyscallReturn> {
    let flags = SendRecvFlags::from_bits_truncate(flags);
    let socket_addr = if dest_addr == 0 {
        None
    } else {
        let socket_addr = read_socket_addr_from_user(dest_addr, addrlen)?;
        Some(socket_addr)
    };
    debug!("sockfd = {sockfd}, buf = 0x{buf:x}, len = 0x{len:x}, flags = {flags:?}, socket_addr = {socket_addr:?}");

    let socket = get_socket_from_fd(sockfd)?;

    let io_vecs = [IoVec::new(buf, len)];
    let message_header = MessageHeader::new(socket_addr, None);

    let send_size = socket.sendmsg(&io_vecs, message_header, flags)?;

    Ok(SyscallReturn::Return(send_size as _))
}
