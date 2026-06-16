// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;

use super::{RawSocketOption, SocketOption, impl_raw_socket_option};
use crate::{net::socket::ip::options::Ipv6V6only, prelude::*};

/// Socket options for IPv6 socket.
///
/// The raw definitions can be found at:
/// <https://elixir.bootlin.com/linux/v6.0.19/source/include/uapi/linux/in6.h#L170>.
#[expect(non_camel_case_types)]
#[repr(i32)]
#[derive(Clone, Copy, Debug, TryFromInt)]
pub enum CIpv6OptionName {
    IPV6_V6ONLY = 26,
}

pub fn new_ipv6_option(name: i32) -> Result<Box<dyn RawSocketOption>> {
    let name = CIpv6OptionName::try_from(name).map_err(|_| Errno::ENOPROTOOPT)?;
    match name {
        CIpv6OptionName::IPV6_V6ONLY => Ok(Box::new(Ipv6V6only::new())),
    }
}

impl_raw_socket_option!(Ipv6V6only);
