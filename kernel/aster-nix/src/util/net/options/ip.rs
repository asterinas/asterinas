// SPDX-License-Identifier: MPL-2.0

use aster_rights::Full;

use super::RawSocketOption;
use crate::{
    impl_raw_socket_option,
    net::socket::ip::options::{RecvErr, RecvTtl, RetOpts},
    prelude::*,
    util::net::options::SocketOption,
    vm::vmar::Vmar,
};

/// Socket level IP options.
///
/// The definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/in.h#L94
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt, PartialEq, Eq, PartialOrd, Ord)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
enum CIpOptionName {
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
}

pub fn new_ip_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CIpOptionName::try_from(name)?;
    match name {
        CIpOptionName::RETOPTS => Ok(Box::new(RetOpts::new())),
        CIpOptionName::RECVERR => Ok(Box::new(RecvErr::new())),
        CIpOptionName::RECVTTL => Ok(Box::new(RecvTtl::new())),
        _ => todo!(),
    }
}

impl_raw_socket_option!(RetOpts);
impl_raw_socket_option!(RecvErr);
impl_raw_socket_option!(RecvTtl);
