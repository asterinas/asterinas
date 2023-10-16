use crate::impl_raw_sock_option;
use crate::net::socket::ip::tcp_options::{TcpCongestion, TcpMaxseg, TcpNoDelay, TcpWindowClamp};
use crate::prelude::*;
use crate::util::net::options::SockOption;
use crate::vm::vmar::Vmar;
use aster_rights::Full;

use super::utils::{read_bool, read_congestion, write_bool, write_congestion};
use super::RawSockOption;

/// Sock options for tcp socket.
///
/// The raw definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/tcp.h#L92
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
pub enum TcpOptionName {
    NODELAY = 1,       /* Turn off Nagle's algorithm. */
    MAXSEG = 2,        /* Limit MSS */
    CORK = 3,          /* Never send partially complete segments */
    KEEPIDLE = 4,      /* Start keeplives after this period */
    KEEPALIVE = 5,     /* Interval between keepalives */
    WINDOW_CLAMP = 10, /* Bound advertised window */
    CONGESTION = 13,   /* Congestion control algorithm */
}

pub fn new_tcp_option(name: i32) -> Result<Box<dyn RawSockOption>> {
    let name = TcpOptionName::try_from(name)?;
    match name {
        TcpOptionName::NODELAY => Ok(Box::new(TcpNoDelay::new())),
        TcpOptionName::CONGESTION => Ok(Box::new(TcpCongestion::new())),
        TcpOptionName::MAXSEG => Ok(Box::new(TcpMaxseg::new())),
        TcpOptionName::WINDOW_CLAMP => Ok(Box::new(TcpWindowClamp::new())),
        _ => todo!(),
    }
}

impl_raw_sock_option!(TcpNoDelay, read_bool, write_bool);
impl_raw_sock_option!(TcpCongestion, read_congestion, write_congestion);
impl_raw_sock_option!(TcpMaxseg);
impl_raw_sock_option!(TcpWindowClamp);
