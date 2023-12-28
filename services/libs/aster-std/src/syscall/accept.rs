use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::net::{get_socket_from_fd, write_socket_addr_to_user};

use super::{SyscallReturn, SYS_ACCEPT};

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
        file_table.insert(connected_socket)
    };
    Ok(SyscallReturn::Return(connected_fd as _))
}
