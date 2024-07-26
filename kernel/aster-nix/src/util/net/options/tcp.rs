// SPDX-License-Identifier: MPL-2.0

use super::RawSocketOption;
use crate::{
    impl_raw_socket_option,
    net::socket::ip::stream::options::{Congestion, MaxSegment, NoDelay, WindowClamp},
    prelude::*,
    util::net::options::SocketOption,
};

/// Sock options for tcp socket.
///
/// The raw definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/tcp.h#L92
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
pub enum CTcpOptionName {
    NODELAY = 1,       /* Turn off Nagle's algorithm. */
    MAXSEG = 2,        /* Limit MSS */
    CORK = 3,          /* Never send partially complete segments */
    KEEPIDLE = 4,      /* Start keeplives after this period */
    KEEPALIVE = 5,     /* Interval between keepalives */
    WINDOW_CLAMP = 10, /* Bound advertised window */
    CONGESTION = 13,   /* Congestion control algorithm */
}

pub fn new_tcp_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CTcpOptionName::try_from(name)?;
    match name {
        CTcpOptionName::NODELAY => Ok(Box::new(NoDelay::new())),
        CTcpOptionName::CONGESTION => Ok(Box::new(Congestion::new())),
        CTcpOptionName::MAXSEG => Ok(Box::new(MaxSegment::new())),
        CTcpOptionName::WINDOW_CLAMP => Ok(Box::new(WindowClamp::new())),
        _ => todo!(),
    }
}

impl_raw_socket_option!(NoDelay);
impl_raw_socket_option!(Congestion);
impl_raw_socket_option!(MaxSegment);
impl_raw_socket_option!(WindowClamp);
