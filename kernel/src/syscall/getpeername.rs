// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{get_file_fast, FileDesc},
    prelude::*,
    util::net::write_socket_addr_to_user,
};

pub fn sys_getpeername(
    sockfd: FileDesc,
    addr: Vaddr,
    addrlen_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, addr = 0x{addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let peer_addr = socket.peer_addr()?;
    // FIXME: trunscate write len if addrlen is not big enough
    write_socket_addr_to_user(&peer_addr, addr, addrlen_ptr)?;

    Ok(SyscallReturn::Return(0))
}
