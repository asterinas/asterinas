// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{
        file_table::{get_file_fast, FdFlags, FileDesc},
        utils::{CreationFlags, StatusFlags},
    },
    prelude::*,
    util::net::write_socket_addr_to_user,
};

pub fn sys_accept(
    sockfd: FileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!("sockfd = {sockfd}, sockaddr_ptr = 0x{sockaddr_ptr:x}, addrlen_ptr = 0x{addrlen_ptr:x}");

    let fd = do_accept(sockfd, sockaddr_ptr, addrlen_ptr, Flags::empty(), ctx)?;
    Ok(SyscallReturn::Return(fd as _))
}

pub fn sys_accept4(
    sockfd: FileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    trace!("raw flags = 0x{:x}", flags);
    let flags = Flags::from_bits_truncate(flags);
    debug!(
        "sockfd = {}, sockaddr_ptr = 0x{:x}, addrlen_ptr = 0x{:x}, flags = {:?}",
        sockfd, sockaddr_ptr, addrlen_ptr, flags
    );

    let fd = do_accept(sockfd, sockaddr_ptr, addrlen_ptr, flags, ctx)?;
    Ok(SyscallReturn::Return(fd as _))
}

fn do_accept(
    sockfd: FileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    flags: Flags,
    ctx: &Context,
) -> Result<FileDesc> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd);
    let socket = file.as_socket_or_err()?;

    let (connected_socket, socket_addr) = {
        socket.accept().map_err(|err| match err.error() {
            // FIXME: `accept` should not be restarted if a timeout has been set on the socket using `setsockopt`.
            Errno::EINTR => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?
    };

    if flags.contains(Flags::SOCK_NONBLOCK) {
        connected_socket.set_status_flags(StatusFlags::O_NONBLOCK)?;
    }

    let fd_flags = if flags.contains(Flags::SOCK_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    if sockaddr_ptr != 0 {
        write_socket_addr_to_user(&socket_addr, sockaddr_ptr, addrlen_ptr)?;
    }

    let fd = {
        let mut file_table_locked = file_table.unwrap().write();
        file_table_locked.insert(connected_socket, fd_flags)
    };

    Ok(fd)
}

bitflags! {
    struct Flags: u32 {
        const SOCK_NONBLOCK = NONBLOCK;
        const SOCK_CLOEXEC = CLOEXEC;
    }
}

const NONBLOCK: u32 = StatusFlags::O_NONBLOCK.bits();
const CLOEXEC: u32 = CreationFlags::O_CLOEXEC.bits();
