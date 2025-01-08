// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
    util::net::read_socket_addr_from_user,
};

pub fn sys_connect(
    sockfd: FileDesc,
    sockaddr_ptr: Vaddr,
    addr_len: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let socket_addr = read_socket_addr_from_user(sockaddr_ptr, addr_len as _)?;
    debug!("fd = {sockfd}, socket_addr = {socket_addr:?}");

    let mut file_table = ctx.thread_local.file_table().borrow_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    socket
        .connect(socket_addr)
        .map_err(|err| match err.error() {
            // FIXME: `connect` should not be restarted if a timeout has been set on the socket using `setsockopt`.
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    Ok(SyscallReturn::Return(0))
}
