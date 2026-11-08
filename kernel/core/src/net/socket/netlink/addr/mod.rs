// SPDX-License-Identifier: MPL-2.0

#![short_vis_path::add(netlink)]

mod multicast;

pub(crate) use multicast::GroupIdSet;
pub(in netlink) use multicast::MAX_GROUPS;

use crate::{net::socket::util::SocketAddr, prelude::*};

/// The socket address of a netlink socket.
///
/// The address contains the port number for unicast
/// and the group IDs for multicast.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct NetlinkSocketAddr {
    port: PortNum,
    groups: GroupIdSet,
}

impl NetlinkSocketAddr {
    /// Creates a new netlink address.
    pub(crate) const fn new(port: PortNum, groups: GroupIdSet) -> Self {
        Self { port, groups }
    }

    /// Creates a new unspecified address.
    ///
    /// Both the port ID and group numbers are left unspecified.
    ///
    /// Note that an unspecified address can also represent the kernel socket address.
    pub(in netlink) const fn new_unspecified() -> Self {
        Self {
            port: UNSPECIFIED_PORT,
            groups: GroupIdSet::new_empty(),
        }
    }

    /// Returns the port number.
    pub(crate) const fn port(&self) -> PortNum {
        self.port
    }

    /// Returns the group ID set.
    pub(crate) const fn groups(&self) -> GroupIdSet {
        self.groups
    }

    /// Adds some new groups to the address.
    pub(in netlink) fn add_groups(&mut self, groups: GroupIdSet) {
        self.groups.add_groups(groups);
    }
}

impl TryFrom<SocketAddr> for NetlinkSocketAddr {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        match value {
            SocketAddr::Netlink(addr) => Ok(addr),
            _ => return_errno_with_message!(
                Errno::EAFNOSUPPORT,
                "the address is in an unsupported address family"
            ),
        }
    }
}

impl From<NetlinkSocketAddr> for SocketAddr {
    fn from(value: NetlinkSocketAddr) -> Self {
        SocketAddr::Netlink(value)
    }
}

pub(in netlink) type NetlinkProtocolId = u32;
pub(in netlink) type PortNum = u32;

pub(in netlink) const UNSPECIFIED_PORT: PortNum = 0;
