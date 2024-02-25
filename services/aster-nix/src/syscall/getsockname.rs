// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_GETSOCKNAME};
use crate::{
    fs::file_table::FileDescripter,
    log_syscall_entry,
    prelude::*,
    util::net::{get_socket_from_fd, write_socket_addr_to_user},
};

pub fn sys_getsockname(
    sockfd: FileDescripter,
    addr: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_GETSOCKNAME);
    debug!("sockfd = {sockfd}, addr = 0x{addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let socket_addr = {
        let socket = get_socket_from_fd(sockfd)?;
        socket.addr()?
    };

    // FIXME: trunscate write len if addrlen is not big enough
    write_socket_addr_to_user(&socket_addr, addr, addrlen_ptr)?;
    Ok(SyscallReturn::Return(0))
}
