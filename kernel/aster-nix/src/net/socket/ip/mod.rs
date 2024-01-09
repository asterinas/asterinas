// SPDX-License-Identifier: MPL-2.0

use crate::net::iface::{IpAddress, IpEndpoint, Ipv4Address};

mod common;
mod datagram;
pub mod stream;

pub use datagram::DatagramSocket;
pub use stream::StreamSocket;

/// A local endpoint, which indicates that the local endpoint is unspecified.
///
/// According to the Linux man pages and the Linux implementation, `getsockname()` will _not_ fail
/// even if the socket is unbound. Instead, it will return an unspecified socket address. This
/// unspecified endpoint helps with that.
const UNSPECIFIED_LOCAL_ENDPOINT: IpEndpoint =
    IpEndpoint::new(IpAddress::Ipv4(Ipv4Address::UNSPECIFIED), 0);
