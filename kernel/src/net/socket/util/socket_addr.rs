// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{Ipv4Address, PortNum};

use crate::{
    net::socket::{netlink::NetlinkSocketAddr, unix::UnixSocketAddr, vsock::addr::VsockSocketAddr},
    prelude::*,
};

#[derive(Debug, PartialEq, Eq)]
pub enum SocketAddr {
    Unix(UnixSocketAddr),
    IPv4(Ipv4Address, PortNum),
    Netlink(NetlinkSocketAddr),
    Vsock(VsockSocketAddr),
}
