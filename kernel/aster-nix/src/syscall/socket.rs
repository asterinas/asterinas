// SPDX-License-Identifier: MPL-2.0

use super::{SyscallReturn, SYS_SOCKET};
use crate::{
    fs::{file_handle::FileLike, file_table::FdFlags},
    log_syscall_entry,
    net::socket::{
        ip::{DatagramSocket, StreamSocket},
        netlink::{NetlinkFamilyType, NetlinkSocket},
        unix::UnixStreamSocket,
    },
    prelude::*,
    util::net::{CSocketAddrFamily, Protocol, SockFlags, SockType, SOCK_TYPE_MASK},
};

pub fn sys_socket(domain: i32, type_: i32, protocol: i32) -> Result<SyscallReturn> {
    log_syscall_entry!(SYS_SOCKET);
    let domain = CSocketAddrFamily::try_from(domain)?;
    let sock_type = SockType::try_from(type_ & SOCK_TYPE_MASK)?;
    let sock_flags = SockFlags::from_bits_truncate(type_ & !SOCK_TYPE_MASK);

    let is_nonblocking = sock_flags.contains(SockFlags::SOCK_NONBLOCK);
    let file_like = if domain == CSocketAddrFamily::AF_NETLINK {
        // For netlink socket, the `protocol` parameter actually means netlink family type.
        let netlink_family = NetlinkFamilyType::try_from(protocol as u32)?;
        debug!(
            "domain = {:?}, sock_type = {:?}, sock_flags = {:?}, family = {:?}",
            domain, sock_type, sock_flags, netlink_family
        );

        match sock_type {
            SockType::SOCK_DGRAM | SockType::SOCK_RAW => {
                Arc::new(NetlinkSocket::new(is_nonblocking, netlink_family)) as Arc<dyn FileLike>
            }
            _ => return_errno_with_message!(
                Errno::EINVAL,
                "netlink socket can only be datagram or raw socket"
            ),
        }
    } else {
        let protocol = Protocol::try_from(protocol)?;
        debug!(
            "domain = {:?}, sock_type = {:?}, sock_flags = {:?}, protocol = {:?}",
            domain, sock_type, sock_flags, protocol
        );

        match (domain, sock_type, protocol) {
            (CSocketAddrFamily::AF_UNIX, SockType::SOCK_STREAM, _) => {
                Arc::new(UnixStreamSocket::new(is_nonblocking)) as Arc<dyn FileLike>
            }
            (
                CSocketAddrFamily::AF_INET,
                SockType::SOCK_STREAM,
                Protocol::IPPROTO_IP | Protocol::IPPROTO_TCP,
            ) => StreamSocket::new(is_nonblocking) as Arc<dyn FileLike>,
            (
                CSocketAddrFamily::AF_INET,
                SockType::SOCK_DGRAM,
                Protocol::IPPROTO_IP | Protocol::IPPROTO_UDP,
            ) => DatagramSocket::new(is_nonblocking) as Arc<dyn FileLike>,
            _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "unsupported domain"),
        }
    };
    let fd = {
        let current = current!();
        let mut file_table = current.file_table().lock();
        let fd_flags = if sock_flags.contains(SockFlags::SOCK_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        file_table.insert(file_like, fd_flags)
    };
    Ok(SyscallReturn::Return(fd as _))
}
