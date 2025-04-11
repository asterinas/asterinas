// SPDX-License-Identifier: MPL-2.0

use super::CSocketAddrFamily;
use crate::{
    net::socket::netlink::{GroupIdSet, NetlinkSocketAddr},
    prelude::*,
};

/// Netlink socket address.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CSocketAddrNetlink {
    /// Address family (AF_NETLINK).
    nl_family: u16,
    /// Pad bytes (always zero).
    nl_pad: u16,
    /// Port ID.
    nl_pid: u32,
    /// Multicast groups mask.
    nl_groups: u32,
}

impl From<NetlinkSocketAddr> for CSocketAddrNetlink {
    fn from(value: NetlinkSocketAddr) -> Self {
        Self {
            nl_family: CSocketAddrFamily::AF_NETLINK as _,
            nl_pad: 0,
            nl_pid: value.port(),
            nl_groups: value.groups().as_u32(),
        }
    }
}

impl From<CSocketAddrNetlink> for NetlinkSocketAddr {
    fn from(value: CSocketAddrNetlink) -> Self {
        debug_assert_eq!(value.nl_family, CSocketAddrFamily::AF_NETLINK as u16);
        let port = value.nl_pid;
        let groups = GroupIdSet::new(value.nl_groups);
        NetlinkSocketAddr::new(port, groups)
    }
}
