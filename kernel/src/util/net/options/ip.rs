// SPDX-License-Identifier: MPL-2.0

use super::OrigSocketOption;
use crate::{
    impl_orig_socket_option,
    net::socket::options::{IpHdrIncl, IpMtu},
    prelude::*,
    util::net::options::SocketOption,
};

/// Ip options for ip level.
///
/// The raw definition is from https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/tcp.h#L92
#[repr(i32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
#[allow(non_camel_case_types)]
#[allow(clippy::upper_case_acronyms)]
pub enum CIpOptionName {
    IP_HDRINCL = 3, /* Ip header is included in user's packet. */
    IP_MTU = 14,
}

pub fn new_ip_option(name: i32) -> Result<Box<dyn OrigSocketOption>> {
    let name = CIpOptionName::try_from(name)?;
    match name {
        CIpOptionName::IP_HDRINCL => Ok(Box::new(IpHdrIncl::new())),
        _ => todo!(),
    }
}

impl_orig_socket_option!(IpHdrIncl);
impl_orig_socket_option!(IpMtu);
