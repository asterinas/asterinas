// SPDX-License-Identifier: MPL-2.0

use crate::fs::file_table::FileDescripter;
use crate::get_socket_without_holding_filetable_lock;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::net::read_socket_addr_from_user;

use super::SyscallReturn;
use super::SYS_CONNECT;

pub fn sys_connect(
    sockfd: FileDescripter,
    sockaddr_ptr: Vaddr,
    addr_len: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_CONNECT);
    let socket_addr = read_socket_addr_from_user(sockaddr_ptr, addr_len as _)?;
    debug!("fd = {sockfd}, socket_addr = {socket_addr:?}");
    let current = current!();
    get_socket_without_holding_filetable_lock!(socket, current, sockfd);
    socket.connect(socket_addr)?;
    Ok(SyscallReturn::Return(0))
}
