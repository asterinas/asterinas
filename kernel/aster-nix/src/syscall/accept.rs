// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_ACCEPT};
use crate::{
    fs::file_table::{FdFlags, FileDescripter},
    log_syscall_entry,
    prelude::*,
    util::net::{get_socket_from_fd, write_socket_addr_to_user},
};

pub fn sys_accept(
    sockfd: FileDescripter,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ACCEPT);
    debug!("sockfd = {sockfd}, sockaddr_ptr = 0x{sockaddr_ptr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let socket = get_socket_from_fd(sockfd)?;

    let (connected_socket, socket_addr) = socket.accept()?;
    write_socket_addr_to_user(&socket_addr, sockaddr_ptr, addrlen_ptr)?;

    let connected_fd = {
        let current = current!();
        let mut file_table = current.file_table().lock();
        file_table.insert(connected_socket, FdFlags::empty())
    };
    Ok(SyscallReturn::Return(connected_fd as _))
}
