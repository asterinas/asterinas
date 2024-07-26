// SPDX-License-Identifier: MPL-2.0

use crate::{
    net::socket::{
        ip::{Ipv4Address, PortNum},
        unix::UnixSocketAddr,
        vsock::addr::VsockSocketAddr,
    },
    prelude::*,
};

#[derive(Debug, PartialEq, Eq)]
pub enum SocketAddr {
    Unix(UnixSocketAddr),
    IPv4(Ipv4Address, PortNum),
    IPv6,
    Vsock(VsockSocketAddr),
}
