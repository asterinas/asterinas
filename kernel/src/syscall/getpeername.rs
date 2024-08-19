// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::FileDesc,
    prelude::*,
    util::net::{get_socket_from_fd, write_socket_addr_to_user},
};

pub fn sys_getpeername(
    sockfd: FileDesc,
    addr: Vaddr,
    addrlen_ptr: Vaddr,
    _ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, addr = 0x{addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let peer_addr = {
        let socket = get_socket_from_fd(sockfd)?;
        socket.peer_addr()?
    };
    // FIXME: trunscate write len if addrlen is not big enough
    write_socket_addr_to_user(&peer_addr, addr, addrlen_ptr)?;
    Ok(SyscallReturn::Return(0))
}
