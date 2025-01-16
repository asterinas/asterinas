// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

use super::RawSocketOption;
use crate::{
    impl_raw_socket_option,
    net::socket::ip::options::{Hdrincl, Tos, Ttl},
    prelude::*,
    util::net::options::SocketOption,
};

/// Socket options for IP socket.
///
/// The raw definitions can be found at:
/// https://elixir.bootlin.com/linux/v6.0.19/source/include/uapi/linux/in.h#L94
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[expect(non_camel_case_types)]
#[expect(clippy::upper_case_acronyms)]
pub enum CIpOptionName {
    TOS = 1,
    TTL = 2,
    HDRINCL = 3,
    OPTIONS = 4,
    ROUTER_ALERT = 5,
    RECVOPTS = 6,
    RETOPTS = 7,
    PKTINFO = 8,
    PKTOPTIONS = 9,
    MTU_DISCOVER = 10,
    RECVERR = 11,
    RECVTTL = 12,
    RECVTOS = 13,
    MTU = 14,
    FREEBIND = 15,
    IPSEC_POLICY = 16,
    XFRM_POLICY = 17,
    PASSSEC = 18,
    TRANSPARENT = 19,
    ORIGDSTADDR = 20,
    MINTTL = 21,
    NODEFRAG = 22,
    CHECKSUM = 23,
    BIND_ADDRESS_NO_PORT = 24,
    RECVFRAGSIZE = 25,
    RECVERR_RFC4884 = 26,
    MULTICAST_IF = 32,
    MULTICAST_TTL = 33,
    MULTICAST_LOOP = 34,
    ADD_MEMBERSHIP = 35,
    DROP_MEMBERSHIP = 36,
    UNBLOCK_SOURCE = 37,
    BLOCK_SOURCE = 38,
    ADD_SOURCE_MEMBERSHIP = 39,
    DROP_SOURCE_MEMBERSHIP = 40,
    MSFILTER = 41,
    MCAST_JOIN_GROUP = 42,
    MCAST_BLOCK_SOURCE = 43,
    MCAST_UNBLOCK_SOURCE = 44,
    MCAST_LEAVE_GROUP = 45,
    MCAST_JOIN_SOURCE_GROUP = 46,
    MCAST_LEAVE_SOURCE_GROUP = 47,
    MCAST_MSFILTER = 48,
    MULTICAST_ALL = 49,
    UNICAST_IF = 50,
}

pub fn new_ip_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CIpOptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CIpOptionName::TOS => Ok(Box::new(Tos::new())),
        CIpOptionName::TTL => Ok(Box::new(Ttl::new())),
        CIpOptionName::HDRINCL => Ok(Box::new(Hdrincl::new())),
        _ => return_errno_with_message!(Errno::ENOPROTOOPT, "unsupported ip level option"),
    }
}

impl_raw_socket_option!(Ttl);
impl_raw_socket_option!(Tos);
impl_raw_socket_option!(Hdrincl);
