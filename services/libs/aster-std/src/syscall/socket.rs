use crate::fs::file_handle::FileLike;
use crate::net::socket::ip::{DatagramSocket, StreamSocket};
use crate::net::socket::unix::UnixStreamSocket;
use crate::util::net::{CSocketAddrFamily, Protocol, SockFlags, SockType, SOCK_TYPE_MASK};
use crate::{log_syscall_entry, prelude::*};

use super::SyscallReturn;
use super::SYS_SOCKET;

pub fn sys_socket(domain: i32, type_: i32, protocol: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SOCKET);
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
        (CSocketAddrFamily::AF_UNIX, SockType::SOCK_STREAM, _) => Arc::new(UnixStreamSocket::new(
            sock_flags.contains(SockFlags::SOCK_NONBLOCK),
        )) as Arc<dyn FileLike>,
        (
            CSocketAddrFamily::AF_INET,
            SockType::SOCK_STREAM,
            Protocol::IPPROTO_IP | Protocol::IPPROTO_TCP,
        ) => Arc::new(StreamSocket::new(nonblocking)) as Arc<dyn FileLike>,
        (
            CSocketAddrFamily::AF_INET,
            SockType::SOCK_DGRAM,
            Protocol::IPPROTO_IP | Protocol::IPPROTO_UDP,
        ) => Arc::new(DatagramSocket::new(nonblocking)) as Arc<dyn FileLike>,
        _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "unsupported domain"),
    };
    let fd = {
        let current = current!();
        let mut file_table = current.file_table().lock();
        file_table.insert(file_like)
    };
    Ok(SyscallReturn::Return(fd as _))
}
