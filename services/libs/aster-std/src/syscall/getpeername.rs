// SPDX-License-Identifier: MPL-2.0

use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::prelude::*;
use crate::util::net::{get_socket_from_fd, write_socket_addr_to_user};

use super::{SyscallReturn, SYS_GETPEERNAME};

pub fn sys_getpeername(
    sockfd: FileDescripter,
    addr: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETPEERNAME);
    debug!("sockfd = {sockfd}, addr = 0x{addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let peer_addr = {
        let socket = get_socket_from_fd(sockfd)?;
        socket.peer_addr()?
    };
    // FIXME: trunscate write len if addrlen is not big enough
    write_socket_addr_to_user(&peer_addr, addr, addrlen_ptr)?;
    Ok(SyscallReturn::Return(0))
}
