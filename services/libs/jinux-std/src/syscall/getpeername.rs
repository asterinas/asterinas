use crate::util::net::write_socket_addr_to_user;
use crate::util::read_val_from_user;
use crate::{fs::file_table::FileDescripter, log_syscall_entry, prelude::*};

use super::SyscallReturn;
use super::SYS_GETPEERNAME;

pub fn sys_getpeername(
    sockfd: FileDescripter,
    addr: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETPEERNAME);
    let addrlen: i32 = read_val_from_user(addrlen_ptr)?;
    debug!("sockfd = {sockfd}, addr = 0x{addr:x}, addrlen = {addrlen}");
    let socket_addr = {
        let current = current!();
        let file_table = current.file_table().lock();
        let socket = file_table.get_socket(sockfd)?;
        socket.peer_addr()?
    };
    // FIXME: trunscate write len if addrlen is not big enough
    write_socket_addr_to_user(&socket_addr, addr, addrlen as usize)?;
    Ok(SyscallReturn::Return(0))
}
