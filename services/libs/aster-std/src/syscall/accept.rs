use crate::util::net::write_socket_addr_to_user;
use crate::{fs::file_table::FileDescripter, prelude::*};
use crate::{get_socket_without_holding_filetable_lock, log_syscall_entry};

use super::SyscallReturn;
use super::SYS_ACCEPT;

pub fn sys_accept(
    sockfd: FileDescripter,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ACCEPT);
    debug!("sockfd = {sockfd}, sockaddr_ptr = 0x{sockaddr_ptr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");
    let current = current!();
    get_socket_without_holding_filetable_lock!(socket, current, sockfd);
    let (connected_socket, socket_addr) = socket.accept()?;
    write_socket_addr_to_user(&socket_addr, sockaddr_ptr, addrlen_ptr)?;
    let fd = {
        let mut file_table = current.file_table().lock();
        file_table.insert(connected_socket, false)
    };
    Ok(SyscallReturn::Return(fd as _))
}
