// SPDX-License-Identifier: MPL-2.0

use super::RawSocketOption;
use crate::{
    impl_raw_sock_option_get_only, impl_raw_sock_option_set_only, impl_raw_socket_option,
    net::socket::options::{
        AcceptConn, AttachFilter, Error, KeepAlive, Linger, PassCred, PeerCred, PeerGroups,
        Priority, RecvBuf, RecvBufForce, ReuseAddr, ReusePort, SendBuf, SendBufForce, SocketOption,
    },
    prelude::*,
};

/// Socket level options.
///
/// The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/socket.h.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
enum CSocketOptionName {
    DEBUG = 1,
    REUSEADDR = 2,
    TYPE = 3,
    ERROR = 4,
    DONTROUTE = 5,
    BROADCAST = 6,
    SNDBUF = 7,
    RCVBUF = 8,
    KEEPALIVE = 9,
    OOBINLINE = 10,
    NO_CHECK = 11,
    PRIORITY = 12,
    LINGER = 13,
    BSDCOMPAT = 14,
    REUSEPORT = 15,
    PASSCRED = 16,
    PEERCRED = 17,
    ATTACH_FILTER = 26,
    DETACH_FILTER = 27,
    ACCPETCONN = 30,
    PEERSEC = 31,
    SNDBUFFORCE = 32,
    RCVBUFFORCE = 33,
    PEERGROUPS = 59,
    RCVTIMEO_NEW = 66,
    SNDTIMEO_NEW = 67,
}

pub fn new_socket_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CSocketOptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CSocketOptionName::SNDBUF => Ok(Box::new(SendBuf::new())),
        CSocketOptionName::RCVBUF => Ok(Box::new(RecvBuf::new())),
        CSocketOptionName::REUSEADDR => Ok(Box::new(ReuseAddr::new())),
        CSocketOptionName::ERROR => Ok(Box::new(Error::new())),
        CSocketOptionName::REUSEPORT => Ok(Box::new(ReusePort::new())),
        CSocketOptionName::PRIORITY => Ok(Box::new(Priority::new())),
        CSocketOptionName::LINGER => Ok(Box::new(Linger::new())),
        CSocketOptionName::KEEPALIVE => Ok(Box::new(KeepAlive::new())),
        CSocketOptionName::PASSCRED => Ok(Box::new(PassCred::new())),
        CSocketOptionName::PEERCRED => Ok(Box::new(PeerCred::new())),
        CSocketOptionName::ATTACH_FILTER => Ok(Box::new(AttachFilter::new())),
        CSocketOptionName::ACCPETCONN => Ok(Box::new(AcceptConn::new())),
        CSocketOptionName::SNDBUFFORCE => Ok(Box::new(SendBufForce::new())),
        CSocketOptionName::RCVBUFFORCE => Ok(Box::new(RecvBufForce::new())),
        CSocketOptionName::PEERGROUPS => Ok(Box::new(PeerGroups::new())),
        _ => return_errno_with_message!(Errno::ENOPROTOOPT, "unsupported socket-level option"),
    }
}

impl_raw_socket_option!(SendBuf);
impl_raw_socket_option!(RecvBuf);
impl_raw_socket_option!(ReuseAddr);
impl_raw_sock_option_get_only!(Error);
impl_raw_socket_option!(ReusePort);
impl_raw_socket_option!(Priority);
impl_raw_socket_option!(Linger);
impl_raw_socket_option!(KeepAlive);
impl_raw_socket_option!(PassCred);
impl_raw_sock_option_get_only!(PeerCred);
impl_raw_sock_option_set_only!(AttachFilter);
impl_raw_sock_option_get_only!(AcceptConn);
impl_raw_socket_option!(SendBufForce);
impl_raw_socket_option!(RecvBufForce);
impl_raw_sock_option_get_only!(PeerGroups);
