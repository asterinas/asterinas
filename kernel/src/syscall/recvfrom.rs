// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    net::socket::SendRecvFlags,
    prelude::*,
    util::{
        net::{get_socket_from_fd, write_socket_addr_to_user},
        IoVec,
    },
};

pub fn sys_recvfrom(
    sockfd: FileDesc,
    buf: Vaddr,
    len: usize,
    flags: i32,
    src_addr: Vaddr,
    addrlen_ptr: Vaddr,
    _ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = SendRecvFlags::from_bits_truncate(flags);
    debug!("sockfd = {sockfd}, buf = 0x{buf:x}, len = {len}, flags = {flags:?}, src_addr = 0x{src_addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let socket = get_socket_from_fd(sockfd)?;

    let io_vecs = [IoVec::new(buf, len)];
    let (recv_size, message_header) = socket.recvmsg(&io_vecs, flags)?;

    if let Some(socket_addr) = message_header.addr()
        && src_addr != 0
    {
        write_socket_addr_to_user(socket_addr, src_addr, addrlen_ptr)?;
    }

    Ok(SyscallReturn::Return(recv_size as _))
}
