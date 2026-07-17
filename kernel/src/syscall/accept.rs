// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file::{
        CreationFlags, StatusFlags,
        file_table::{FdFlags, RawFileDesc, get_file_fast},
    },
    prelude::*,
    util::net::write_socket_addr_to_user,
};

/// Accepts a connection on a listening socket.
///
/// This is equivalent to `accept4(sockfd, addr, addrlen, 0)`.
/// See `accept4(2)` for the full specification.
pub fn sys_accept(
    sockfd: RawFileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    debug!(
        "sockfd = {sockfd}, sockaddr_ptr = 0x{sockaddr_ptr:x}, addrlen_ptr = 0x{addrlen_ptr:x}"
    );

    do_accept(sockfd, sockaddr_ptr, addrlen_ptr, Flags::empty(), ctx)
}

/// Accepts a connection on a listening socket, with additional flags.
///
/// Extends `accept(2)` by allowing `SOCK_NONBLOCK` and `SOCK_CLOEXEC` to be
/// set atomically on the new file descriptor, avoiding a separate `fcntl(2)`
/// call. See `accept4(2)` for the full specification.
pub fn sys_accept4(
    sockfd: RawFileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    flags: u32,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let flags = Flags::from_bits(flags)
        .ok_or_else(|| Error::with_message(Errno::EINVAL, "invalid accept4 flags"))?;
    debug!(
        "sockfd = {sockfd}, sockaddr_ptr = 0x{sockaddr_ptr:x}, addrlen_ptr = 0x{addrlen_ptr:x}, flags = {flags:?}"
    );

    do_accept(sockfd, sockaddr_ptr, addrlen_ptr, flags, ctx)
}

/// Core implementation shared by [`sys_accept`] and [`sys_accept4`].
///
/// Waits for an incoming connection on `sockfd`, inserts the new socket into
/// the file table with the requested `flags`, and writes the peer address to
/// user space when `sockaddr_ptr` is non-null.
fn do_accept(
    sockfd: RawFileDesc,
    sockaddr_ptr: Vaddr,
    addrlen_ptr: Vaddr,
    flags: Flags,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let mut file_table = ctx.thread_local.borrow_file_table_mut();
    let file = get_file_fast!(&mut file_table, sockfd.try_into()?);
    let socket = file.as_socket_or_err()?;

    let is_nonblocking = flags.contains(Flags::SOCK_NONBLOCK);

    let (connected_socket, socket_addr) = socket
        .accept(is_nonblocking)
        .map_err(|err| match err.error() {
            // Per accept(2) and restart_syscall(2): `accept` should surface `EINTR`
            // instead of restarting if a timeout has been set via `setsockopt(SO_RCVTIMEO)`.
            Errno::EINTR if socket.recv_timeout().is_none() => Error::new(Errno::ERESTARTSYS),
            _ => err,
        })?;

    let fd_flags = if flags.contains(Flags::SOCK_CLOEXEC) {
        FdFlags::CLOEXEC
    } else {
        FdFlags::empty()
    };

    // Per accept(2): if `sockaddr_ptr` is null, neither the address nor the
    // address length is written to user space. When `sockaddr_ptr` is non-null
    // but `addrlen_ptr` is null, `write_socket_addr_to_user` dereferences it
    // and correctly surfaces `EFAULT`, matching Linux behaviour.
    if sockaddr_ptr != 0 {
        write_socket_addr_to_user(&socket_addr, sockaddr_ptr, addrlen_ptr)?;
    }

    let fd = file_table.unwrap().write().insert(connected_socket, fd_flags);

    Ok(SyscallReturn::Return(fd.into()))
}

bitflags! {
    /// Flags accepted by [`sys_accept4`].
    ///
    /// These flags are applied atomically to the new file descriptor returned
    /// by the call, avoiding a separate `fcntl(2)` invocation.
    struct Flags: u32 {
        /// Sets `O_NONBLOCK` on the new socket file descriptor.
        const SOCK_NONBLOCK = NONBLOCK;
        /// Sets `FD_CLOEXEC` on the new socket file descriptor,
        /// causing it to be closed automatically on `execve(2)`.
        const SOCK_CLOEXEC = CLOEXEC;
    }
}

const NONBLOCK: u32 = StatusFlags::O_NONBLOCK.bits();
const CLOEXEC: u32 = CreationFlags::O_CLOEXEC.bits();
