// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file::{
        StatusFlags,
        file_table::{RawFileDesc, get_file_fast},
    },
    net::socket::util::AcceptFlags,
    prelude::*,
    util::net::write_socket_addr_to_user,
};

pub fn sys_accept(
    sockfd: RawFileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, sockaddr_ptr = 0x{sockaddr_ptr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    do_accept(sockfd, sockaddr_ptr, addrlen_ptr, AcceptFlags::empty(), ctx)
}

pub fn sys_accept4(
    sockfd: RawFileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("raw flags = 0x{:x}", flags);
    let flags = AcceptFlags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid accept4 flags"))?;
    debug!(
        "sockfd = {}, sockaddr_ptr = 0x{:x}, addrlen_ptr = 0x{:x}, flags = {:?}",
        sockfd, sockaddr_ptr, addrlen_ptr, flags
    );

    do_accept(sockfd, sockaddr_ptr, addrlen_ptr, flags, ctx)
}

fn do_accept(
    sockfd: RawFileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    flags: AcceptFlags,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd.try_into()?);
    let socket = file.as_socket_or_err()?;

    let (connected_socket, socket_addr) = {
        socket.accept().map_err(|err| match err.error() {
            // FIXME: `accept` should not be restarted if a timeout has been set on the socket using `setsockopt`.
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?
    };

    if flags.is_nonblocking() {
        connected_socket.set_status_flags(StatusFlags::O_NONBLOCK)?;
    }

    if sockaddr_ptr != 0 {
        write_socket_addr_to_user(&socket_addr, sockaddr_ptr, addrlen_ptr)?;
    }

    let fd = {
        let mut file_table_locked = file_table.unwrap().write();
        file_table_locked.insert(connected_socket, flags.fd_flags())
    };

    Ok(SyscallReturn::Return(fd.into()))
}
