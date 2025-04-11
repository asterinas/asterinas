// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
    util::net::read_socket_addr_from_user,
};

pub fn sys_bind(
    sockfd: FileDesc,
    sockaddr_ptr: Vaddr,
    addrlen: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let socket_addr = read_socket_addr_from_user(sockaddr_ptr, addrlen as usize)?;
    debug!("sockfd = {sockfd}, socket_addr = {socket_addr:?}");

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    socket.bind(socket_addr)?;

    Ok(SyscallReturn::Return(0))
}
