// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    net::socket::util::SendRecvFlags,
    prelude::*,
    util::net::write_socket_addr_to_user,
};

pub fn sys_recvfrom(
    sockfd: FileDesc,
    buf: Vaddr,
    len: usize,
    flags: i32,
    src_addr: Vaddr,
    addrlen_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = SendRecvFlags::from_bits_truncate(flags);
    debug!("sockfd = {sockfd}, buf = 0x{buf:x}, len = {len}, flags = {flags:?}, src_addr = 0x{src_addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let user_space = ctx.user_space();
    let mut writers = user_space.writer(buf, len)?;

    let (recv_size, message_header) =
        socket
            .recvmsg(&mut writers, flags)
            .map_err(|err| match err.error() {
                // FIXME: `recvfrom` should not be restarted if a timeout has been set on the socket using `setsockopt`.
                Errno::EINTR => Error::new(Errno::ERESTARTSYS),
                _ => err,
            })?;

    if let Some(socket_addr) = message_header.addr()
        && src_addr != 0
    {
        write_socket_addr_to_user(socket_addr, src_addr, addrlen_ptr)?;
    }

    Ok(SyscallReturn::Return(recv_size as _))
}
