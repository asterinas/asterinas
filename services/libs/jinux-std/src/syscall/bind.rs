use crate::log_syscall_entry;
use crate::util::net::read_socket_addr_from_user;
use crate::{fs::file_table::FileDescripter, prelude::*};

use super::SyscallReturn;
use super::SYS_BIND;

pub fn sys_bind(
    sockfd: FileDescripter,
    sockaddr_ptr: Vaddr,
    addrlen: u32,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_BIND);
    let socket_addr = read_socket_addr_from_user(sockaddr_ptr, addrlen as usize)?;
    debug!("sockfd = {sockfd}, socket_addr = {socket_addr:?}");
    let current = current!();
    let file_table = current.file_table().lock();
    let socket = file_table.get_socket(sockfd)?;
    socket.bind(socket_addr)?;
    Ok(SyscallReturn::Return(0))
}
