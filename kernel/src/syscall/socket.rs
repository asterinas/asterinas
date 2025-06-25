// SPDX-License-Identifier: MPL-2.0

use super::SyscallReturn;
use crate::{
    fs::{file_handle::FileLike, file_table::FdFlags},
    net::socket::{
        ip::{DatagramSocket, StreamSocket},
        netlink::{
            is_valid_protocol, NetlinkRouteSocket, NetlinkUeventSocket, StandardNetlinkProtocol,
        },
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
    debug!(
        "domain = {:?}, sock_type = {:?}, sock_flags = {:?}",
        domain, sock_type, sock_flags
    );

    let is_nonblocking = sock_flags.contains(SockFlags::SOCK_NONBLOCK);
    let file_like = match (domain, sock_type) {
        (CSocketAddrFamily::AF_UNIX, SockType::SOCK_STREAM) => {
            UnixStreamSocket::new(is_nonblocking, false) as Arc<dyn FileLike>
        }
        (CSocketAddrFamily::AF_UNIX, SockType::SOCK_SEQPACKET) => {
            UnixStreamSocket::new(is_nonblocking, true) as Arc<dyn FileLike>
        }
        (CSocketAddrFamily::AF_INET, SockType::SOCK_STREAM) => {
            let protocol = Protocol::try_from(protocol)?;
            debug!("protocol = {:?}", protocol);
            match protocol {
                Protocol::IPPROTO_IP | Protocol::IPPROTO_TCP => {
                    StreamSocket::new(is_nonblocking) as Arc<dyn FileLike>
                }
                _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "unsupported protocol"),
            }
        }
        (CSocketAddrFamily::AF_INET, SockType::SOCK_DGRAM) => {
            let protocol = Protocol::try_from(protocol)?;
            debug!("protocol = {:?}", protocol);
            match protocol {
                Protocol::IPPROTO_IP | Protocol::IPPROTO_UDP => {
                    DatagramSocket::new(is_nonblocking) as Arc<dyn FileLike>
                }
                _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "unsupported protocol"),
            }
        }
        (CSocketAddrFamily::AF_NETLINK, SockType::SOCK_RAW | SockType::SOCK_DGRAM) => {
            let netlink_family = StandardNetlinkProtocol::try_from(protocol as u32);
            debug!("netlink family = {:?}", netlink_family);
            match netlink_family {
                Ok(StandardNetlinkProtocol::ROUTE) => {
                    NetlinkRouteSocket::new(is_nonblocking) as Arc<dyn FileLike>
                }
                Ok(StandardNetlinkProtocol::KOBJECT_UEVENT) => {
                    NetlinkUeventSocket::new(is_nonblocking) as Arc<dyn FileLike>
                }
                Ok(_) => {
                    return_errno_with_message!(
                        Errno::EAFNOSUPPORT,
                        "some standard netlink families are not supported yet"
                    );
                }
                Err(_) => {
                    if is_valid_protocol(protocol as u32) {
                        return_errno_with_message!(
                            Errno::EAFNOSUPPORT,
                            "user-provided netlink family is not supported"
                        )
                    }
                    return_errno_with_message!(Errno::EAFNOSUPPORT, "invalid netlink family");
                }
            }
        }
        (CSocketAddrFamily::AF_VSOCK, SockType::SOCK_STREAM) => {
            Arc::new(VsockStreamSocket::new(is_nonblocking)?) as Arc<dyn FileLike>
        }
        _ => return_errno_with_message!(Errno::EAFNOSUPPORT, "unsupported domain"),
    };

    let fd = {
        let file_table = ctx.thread_local.borrow_file_table();
        let mut file_table_locked = file_table.unwrap().write();
        let fd_flags = if sock_flags.contains(SockFlags::SOCK_CLOEXEC) {
            FdFlags::CLOEXEC
        } else {
            FdFlags::empty()
        };
        file_table_locked.insert(file_like, fd_flags)
    };

    Ok(SyscallReturn::Return(fd as _))
}
