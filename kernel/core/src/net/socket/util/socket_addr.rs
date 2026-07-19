// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{Ipv4Address, Ipv6Address, PortNum};

use crate::{
    net::socket::{netlink::NetlinkSocketAddr, unix::UnixSocketAddr, vsock::VsockSocketAddr},
    prelude::*,
};

#[derive(Debug, Eq, PartialEq)]
pub enum SocketAddr {
    Unix(UnixSocketAddr),
    IPv4(Ipv4Address, PortNum),
    IPv6(Ipv6Address, PortNum),
    Netlink(NetlinkSocketAddr),
    Vsock(VsockSocketAddr),
}
