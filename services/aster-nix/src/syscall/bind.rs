// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_BIND};
use crate::{
    fs::file_table::FileDescripter,
    log_syscall_entry,
    prelude::*,
    util::net::{get_socket_from_fd, read_socket_addr_from_user},
};

pub fn sys_bind(
    sockfd: FileDescripter,
    sockaddr_ptr: Vaddr,
    addrlen: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_BIND);
    let socket_addr = read_socket_addr_from_user(sockaddr_ptr, addrlen as usize)?;
    debug!("sockfd = {sockfd}, socket_addr = {socket_addr:?}");

    let socket = get_socket_from_fd(sockfd)?;
    socket.bind(socket_addr)?;
    Ok(SyscallReturn::Return(0))
}
