use crate::net::socket::options::{
    SockOption, SocketError, SocketLinger, SocketRecvBuf, SocketReuseAddr, SocketReusePort,
    SocketSendBuf,
};
use crate::prelude::*;
use crate::vm::vmar::Vmar;
use crate::{impl_raw_sock_option, impl_raw_sock_option_get_only};
use aster_rights::Full;

use super::utils::{read_bool, read_linger, write_bool, write_errors, write_linger};
use super::RawSockOption;

/// Socket level options.
///
/// The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/asm-generic/socket.h.
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
enum SocketOptionName {
    DEBUG = 1,
    REUSEADDR = 2,
    TYPE = 3,
    ERROR = 4,
    DONTROUTE = 5,
    BROADCAST = 6,
    SNDBUF = 7,
    RCVBUF = 8,
    SNDBUFFORCE = 32,
    RCVBUFFORCE = 33,
    KEEPALIVE = 9,
    OOBINLINE = 10,
    NO_CHECK = 11,
    PRIORITY = 12,
    LINGER = 13,
    BSDCOMPAT = 14,
    REUSEPORT = 15,
    RCVTIMEO_NEW = 66,
    SNDTIMEO_NEW = 67,
}

pub fn new_socket_option(name: i32) -> Result<Box<dyn RawSockOption>> {
    let name = SocketOptionName::try_from(name)?;
    match name {
        SocketOptionName::SNDBUF => Ok(Box::new(SocketSendBuf::new())),
        SocketOptionName::RCVBUF => Ok(Box::new(SocketRecvBuf::new())),
        SocketOptionName::REUSEADDR => Ok(Box::new(SocketReuseAddr::new())),
        SocketOptionName::ERROR => Ok(Box::new(SocketError::new())),
        SocketOptionName::REUSEPORT => Ok(Box::new(SocketReusePort::new())),
        SocketOptionName::LINGER => Ok(Box::new(SocketLinger::new())),
        _ => todo!(),
    }
}

impl_raw_sock_option!(SocketSendBuf);
impl_raw_sock_option!(SocketRecvBuf);
impl_raw_sock_option!(SocketReuseAddr, read_bool, write_bool);
impl_raw_sock_option_get_only!(SocketError, write_errors);
impl_raw_sock_option!(SocketReusePort, read_bool, write_bool);
impl_raw_sock_option!(SocketLinger, read_linger, write_linger);
