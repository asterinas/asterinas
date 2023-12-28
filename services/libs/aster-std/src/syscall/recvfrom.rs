use crate::fs::file_table::FileDescripter;
use crate::log_syscall_entry;
use crate::net::socket::SendRecvFlags;
use crate::prelude::*;
use crate::util::net::{get_socket_from_fd, write_socket_addr_to_user};
use crate::util::write_bytes_to_user;

use super::{SyscallReturn, SYS_RECVFROM};

pub fn sys_recvfrom(
    sockfd: FileDescripter,
    buf: Vaddr,
    len: usize,
    flags: i32,
    src_addr: Vaddr,
    addrlen_ptr: Vaddr,
) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_RECVFROM);
    let flags = SendRecvFlags::from_bits_truncate(flags);
    debug!("sockfd = {sockfd}, buf = 0x{buf:x}, len = {len}, flags = {flags:?}, src_addr = 0x{src_addr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let socket = get_socket_from_fd(sockfd)?;

    let mut buffer = vec![0u8; len];

    let (recv_size, socket_addr) = socket.recvfrom(&mut buffer, flags)?;
    if buf != 0 {
        write_bytes_to_user(buf, &buffer[..recv_size])?;
    }
    if src_addr != 0 {
        write_socket_addr_to_user(&socket_addr, src_addr, addrlen_ptr)?;
    }
    Ok(SyscallReturn::Return(recv_size as _))
}
