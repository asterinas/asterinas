use crate::util::net::write_socket_addr_to_user;
use crate::util::read_val_from_user;
use crate::{fs::file_table::FileDescripter, prelude::*};
use crate::{get_socket_without_holding_filetable_lock, log_syscall_entry};

use super::SyscallReturn;
use super::SYS_ACCEPT;

pub fn sys_accept(
    sockfd: FileDescripter,
    sockaddr_ptr: Vaddr,
    addr_len_ptr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_ACCEPT);
    let addr_len: i32 = read_val_from_user(addr_len_ptr)?;
    debug!("sockfd = {sockfd}, sockaddr_ptr = 0x{sockaddr_ptr:x}, addr_len = {addr_len}");
    let current = current!();
    get_socket_without_holding_filetable_lock!(socket, current, sockfd);
    let (connected_socket, socket_addr) = socket.accept()?;
    write_socket_addr_to_user(&socket_addr, sockaddr_ptr, addr_len as usize)?;
    let fd = {
        let mut file_table = current.file_table().lock();
        file_table.insert(connected_socket)
    };
    Ok(SyscallReturn::Return(fd as _))
}
