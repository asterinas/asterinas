// SPDX-License-Identifier: MPL-2.0

mod multicast;

pub use multicast::{GroupIdSet, MAX_GROUPS};

use crate::{net::socket::util::SocketAddr, prelude::*};

/// The socket address of a netlink socket.
///
/// The address contains the port number for unicast
/// and the group IDs for multicast.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct NetlinkSocketAddr {
    port: PortNum,
    groups: GroupIdSet,
}

impl NetlinkSocketAddr {
    /// Creates a new netlink address.
    pub const fn new(port: PortNum, groups: GroupIdSet) -> Self {
        Self { port, groups }
    }

    /// Creates a new unspecified address.
    ///
    /// Both the port ID and group numbers are left unspecified.
    ///
    /// Note that an unspecified address can also represent the kernel socket address.
    pub const fn new_unspecified() -> Self {
        Self {
            port: UNSPECIFIED_PORT,
            groups: GroupIdSet::new_empty(),
        }
    }

    /// Returns the port number.
    pub const fn port(&self) -> PortNum {
        self.port
    }

    /// Returns the group ID set.
    pub const fn groups(&self) -> GroupIdSet {
        self.groups
    }

    /// Adds some new groups to the address.
    pub fn add_groups(&mut self, groups: GroupIdSet) {
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

pub type NetlinkProtocolId = u32;
pub type PortNum = u32;

pub const UNSPECIFIED_PORT: PortNum = 0;
