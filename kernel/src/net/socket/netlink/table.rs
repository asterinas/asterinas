// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use super::{
    addr::{NetlinkProtocolId, NetlinkSocketAddr, PortNum},
    multicast_group::{GroupIdSet, MuilicastGroup, MAX_GROUPS},
};
use crate::{net::socket::netlink::addr::UNSPECIFIED_PORT, prelude::*, util::random::getrandom};

pub static NETLINK_SOCKET_TABLE: NetlinkSocketTable = NetlinkSocketTable::new();

/// All bound netlink sockets.
pub struct NetlinkSocketTable {
    protocols: RwMutex<BTreeMap<NetlinkProtocolId, RwMutex<ProtocolSocketTable>>>,
}

impl NetlinkSocketTable {
    pub const fn new() -> Self {
        Self {
            protocols: RwMutex::new(BTreeMap::new()),
        }
    }

    /// Adds a new netlink protocol
    fn add_new_protocol(&self, protocol: NetlinkProtocolId) {
        let mut protocols = self.protocols.write();
        if protocols.contains_key(&protocol) {
            return;
        }
        let new_protocol = RwMutex::new(ProtocolSocketTable::new(protocol));
        protocols.insert(protocol, new_protocol);
    }

    pub fn bind(
        &self,
        protocol: NetlinkProtocolId,
        addr: &NetlinkSocketAddr,
    ) -> Result<BoundHandle> {
        let protocols = self.protocols.read();

        let Some(protocol_sockets) = protocols.get(&protocol) else {
            return_errno_with_message!(Errno::EINVAL, "the netlink protocol does not exist")
        };

        let mut protocol_sockets = protocol_sockets.write();
        protocol_sockets.bind(addr)
    }
}

/// Bound sockets of a single netlink protocol.
///
/// Each protocol has a unique `ProtocolId`(u32).
/// Each protocol can have bound sockcts for unit cast
/// and at most 32 groups for multicast.
struct ProtocolSocketTable {
    id: NetlinkProtocolId,
    unitcast_sockets: BTreeSet<PortNum>,
    multicast_groups: Box<[MuilicastGroup]>,
}

impl ProtocolSocketTable {
    /// Creates a new netlink protocol
    fn new(id: NetlinkProtocolId) -> Self {
        let multicast_groups = (0u32..MAX_GROUPS).map(|_| MuilicastGroup::new()).collect();
        Self {
            id,
            unitcast_sockets: BTreeSet::new(),
            multicast_groups,
        }
    }

    /// Binds a socket to the netlink protocol.
    /// Returns the bound handle.
    ///
    /// The socket will be bound to a port with `port_num`.
    /// If `port_num` is not provided, kernel will assign a port for it,
    /// typically, the port with the process id of current process.
    /// If the port is already used,
    /// this function will try to allocate a random unused port.
    ///
    /// Meanwhile, this socket can join one or more multicast groups,
    /// which is specified in `groups`.
    fn bind(&mut self, addr: &NetlinkSocketAddr) -> Result<BoundHandle> {
        let port = if addr.port() != UNSPECIFIED_PORT {
            addr.port()
        } else {
            let mut random_port = current!().pid();
            while random_port == UNSPECIFIED_PORT || self.unitcast_sockets.contains(&random_port) {
                getrandom(random_port.as_bytes_mut()).unwrap();
            }
            random_port
        };

        if self.unitcast_sockets.contains(&port) {
            return_errno_with_message!(Errno::EADDRINUSE, "try to bind to an used port");
        }

        self.unitcast_sockets.insert(port);

        for group_id in addr.groups().ids_iter() {
            debug_assert!(group_id < MAX_GROUPS);
            let group = &mut self.multicast_groups[group_id as usize];
            group.add_member(port);
        }

        Ok(BoundHandle::new(self.id, port, addr.groups()))
    }
}

/// A bound netlink socket address.
///
/// When dropping a `BoundHandle`, the port will be automatically released.
#[derive(Debug)]
pub struct BoundHandle {
    protocol: NetlinkProtocolId,
    port: PortNum,
    groups: GroupIdSet,
}

impl BoundHandle {
    fn new(protocol: NetlinkProtocolId, port: PortNum, groups: GroupIdSet) -> Self {
        debug_assert_ne!(port, 0);

        Self {
            protocol,
            port,
            groups,
        }
    }

    pub const fn addr(&self) -> NetlinkSocketAddr {
        NetlinkSocketAddr::new(self.port, self.groups)
    }
}

impl Drop for BoundHandle {
    fn drop(&mut self) {
        let protocols = NETLINK_SOCKET_TABLE.protocols.read();
        let mut protocol_sockets = protocols.get(&self.protocol).unwrap().write();
        protocol_sockets.unitcast_sockets.remove(&self.port);

        for group_id in self.groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }
    }
}

pub(super) fn init() {
    for protocol in 0..MAX_ALLOWED_PROTOCOL_ID {
        if is_standard_protocol(protocol) {
            NETLINK_SOCKET_TABLE.add_new_protocol(protocol);
        }
    }
}

/// Returns whether the `protocol` is valid
pub fn is_valid_protocol(protocol: NetlinkProtocolId) -> bool {
    protocol < MAX_ALLOWED_PROTOCOL_ID
}

/// Returns whether the `protocol` has reserved for some system use
pub fn is_standard_protocol(protocol: NetlinkProtocolId) -> bool {
    StandardNetlinkProtocol::try_from(protocol).is_ok()
}

/// These protocols are currently assigned for specific usage.
/// <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/netlink.h#L9>.
#[allow(non_camel_case_types)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum StandardNetlinkProtocol {
    /// Routing/device hook
    NETLINK_ROUTE = 0,
    /// Unused number
    NETLINK_UNUSED = 1,
    /// Reserved for user mode socket protocols
    NETLINK_USERSOCK = 2,
    /// Unused number, formerly ip_queue
    NETLINK_FIREWALL = 3,
    /// socket monitoring
    NETLINK_SOCK_DIAG = 4,
    /// netfilter/iptables ULOG
    NETLINK_NFLOG = 5,
    /// ipsec
    NETLINK_XFRM = 6,
    /// SELinux event notifications
    NETLINK_SELINUX = 7,
    /// Open-iSCSI
    NETLINK_ISCSI = 8,
    /// auditing
    NETLINK_AUDIT = 9,
    NETLINK_FIB_LOOKUP = 10,
    NETLINK_CONNECTOR = 11,
    /// netfilter subsystem
    NETLINK_NETFILTER = 12,
    NETLINK_IP6_FW = 13,
    /// DECnet routing messages
    NETLINK_DNRTMSG = 14,
    /// Kernel messages to userspace
    NETLINK_KOBJECT_UEVENT = 15,
    NETLINK_GENERIC = 16,
    // leave room for NETLINK_DM (DM Events)
    /// SCSI Transports
    NETLINK_SCSITRANSPORT = 18,
    NETLINK_ECRYPTFS = 19,
    NETLINK_RDMA = 20,
    /// Crypto layer
    NETLINK_CRYPTO = 21,
    /// SMC monitoring
    NETLINK_SMC = 22,
}

const MAX_ALLOWED_PROTOCOL_ID: NetlinkProtocolId = 32;
