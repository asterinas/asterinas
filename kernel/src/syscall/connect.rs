// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    prelude::*,
    util::net::{get_socket_from_fd, read_socket_addr_from_user},
};

pub fn sys_connect(
    sockfd: FileDesc,
    sockaddr_ptr: Vaddr,
    addr_len: u32,
    _ctx: &Context,
) -> Result<SyscallReturn> {
    let socket_addr = read_socket_addr_from_user(sockaddr_ptr, addr_len as _)?;
    debug!("fd = {sockfd}, socket_addr = {socket_addr:?}");

    let socket = get_socket_from_fd(sockfd)?;
    socket.connect(socket_addr)?;
    Ok(SyscallReturn::Return(0))
}
