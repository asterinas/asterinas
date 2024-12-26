// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{fs::file_table::FileDesc, prelude::*, util::net::write_socket_addr_to_user};

pub fn sys_getsockname(
    sockfd: FileDesc,
    addr: Vaddr,
    addrlen_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, addr = 0x{addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let file = {
        let file_table = ctx.posix_thread.file_table().lock();
        file_table.get_file(sockfd)?.clone()
    };
    let socket = file.as_socket_or_err()?;

    let socket_addr = socket.addr()?;
    // FIXME: trunscate write len if addrlen is not big enough
    write_socket_addr_to_user(&socket_addr, addr, addrlen_ptr)?;

    Ok(SyscallReturn::Return(0))
}
