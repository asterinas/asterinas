// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_handle::FileLike, file_table::FdFlags},
    net::socket::{
        ip::{DatagramSocket, StreamSocket},
        unix::UnixStreamSocket,
        vsock::VsockStreamSocket,
    },
    prelude::*,
    util::net::{CSocketAddrFamily, Protocol, SockFlags, SockType, SOCK_TYPE_MASK},
};

pub fn sys_socket(domain: i32, type_: i32, protocol: i32, ctx: &Context) -> Result<SyscallReturn> {
    let domain = CSocketAddrFamily::try_from(domain)?;
    let sock_type = SockType::try_from(type_ & SOCK_TYPE_MASK)?;
    let sock_flags = SockFlags::from_bits_truncate(type_ & !SOCK_TYPE_MASK);
    let protocol = Protocol::try_from(protocol)?;
    debug!(
        "domain = {:?}, sock_type = {:?}, sock_flags = {:?}, protocol = {:?}",
        domain, sock_type, sock_flags, protocol
    );
    let nonblocking = sock_flags.contains(SockFlags::SOCK_NONBLOCK);
    let file_like = match (domain, sock_type, protocol) {
        // FIXME: SOCK_SEQPACKET is added to run fcntl_test, not supported yet.
        (CSocketAddrFamily::AF_UNIX, SockType::SOCK_STREAM | SockType::SOCK_SEQPACKET, _) => {
            UnixStreamSocket::new(nonblocking) as Arc<dyn FileLike>
        }
        (
            CSocketAddrFamily::AF_INET,
            SockType::SOCK_STREAM,
            Protocol::IPPROTO_IP | Protocol::IPPROTO_TCP,
        ) => StreamSocket::new(nonblocking) as Arc<dyn FileLike>,
        (
            CSocketAddrFamily::AF_INET,
            SockType::SOCK_DGRAM,
            Protocol::IPPROTO_IP | Protocol::IPPROTO_UDP,
        ) => DatagramSocket::new(nonblocking) as Arc<dyn FileLike>,
        (CSocketAddrFamily::AF_VSOCK, SockType::SOCK_STREAM, _) => {
            Arc::new(VsockStreamSocket::new(nonblocking)) as Arc<dyn FileLike>
        }
        _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "unsupported domain"),
    };
    let fd = {
        let mut file_table = ctx.process.file_table().lock();
        let fd_flags = if sock_flags.contains(SockFlags::SOCK_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        file_table.insert(file_like, fd_flags)
    };
    Ok(SyscallReturn::Return(fd as _))
}
