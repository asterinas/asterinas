// SPDX-License-Identifier: MPL-2.0

use crate::fs::file_table::FileDescripter;
use crate::get_socket_without_holding_filetable_lock;
use crate::log_syscall_entry;
use crate::net::socket::SendRecvFlags;
use crate::prelude::*;
use crate::util::net::read_socket_addr_from_user;
use crate::util::read_bytes_from_user;

use super::SyscallReturn;
use super::SYS_SENDTO;

pub fn sys_sendto(
    sockfd: FileDescripter,
    buf: Vaddr,
    len: usize,
    flags: i32,
    dest_addr: Vaddr,
    addrlen: usize,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SENDTO);
    let flags = SendRecvFlags::from_bits_truncate(flags);
    let socket_addr = if dest_addr == 0 {
        None
    } else {
        let socket_addr = read_socket_addr_from_user(dest_addr, addrlen)?;
        Some(socket_addr)
    };
    debug!("sockfd = {sockfd}, buf = 0x{buf:x}, len = 0x{len:x}, flags = {flags:?}, socket_addr = {socket_addr:?}");
    let mut buffer = vec![0u8; len];
    read_bytes_from_user(buf, &mut buffer)?;
    let current = current!();
    get_socket_without_holding_filetable_lock!(socket, current, sockfd);
    let send_size = socket.sendto(&buffer, socket_addr, flags)?;
    Ok(SyscallReturn::Return(send_size as _))
}
