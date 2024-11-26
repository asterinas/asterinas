// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::file_table::{FdFlags, FileDesc},
    net::socket::unix::UnixStreamSocket,
    prelude::*,
    util::net::{CSocketAddrFamily, Protocol, SockFlags, SockType, SOCK_TYPE_MASK},
};

pub fn sys_socketpair(
    domain: i32,
    type_: i32,
    protocol: i32,
    sv: Vaddr,
    ctx: &Context,
) -> Result<SyscallReturn> {
    let domain = CSocketAddrFamily::try_from(domain)?;
    let sock_type = SockType::try_from(type_ & SOCK_TYPE_MASK)?;
    let sock_flags = SockFlags::from_bits_truncate(type_ & !SOCK_TYPE_MASK);
    let protocol = Protocol::try_from(protocol)?;

    debug!(
        "domain = {:?}, sock_type = {:?}, sock_flags = {:?}, protocol = {:?}",
        domain, sock_type, sock_flags, protocol
    );
    // TODO: deal with all sock_flags and protocol
    let nonblocking = sock_flags.contains(SockFlags::SOCK_NONBLOCK);
    let (socket_a, socket_b) = match (domain, sock_type) {
        (CSocketAddrFamily::AF_UNIX, SockType::SOCK_STREAM) => {
            UnixStreamSocket::new_pair(nonblocking)
        }
        _ => return_errno_with_message!(
            Errno::EAFNOSUPPORT,
            "cannot create socket pair for this family"
        ),
    };

    let socket_fds = {
        let mut file_table = ctx.process.file_table().lock();
        let fd_flags = if sock_flags.contains(SockFlags::SOCK_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        let fd_a = file_table.insert(socket_a, fd_flags);
        let fd_b = file_table.insert(socket_b, fd_flags);
        SocketFds(fd_a, fd_b)
    };

    ctx.user_space().write_val(sv, &socket_fds)?;
    Ok(SyscallReturn::Return(0))
}

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct SocketFds(FileDesc, FileDesc);
