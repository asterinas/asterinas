// SPDX-License-Identifier: MPL-2.0

use multicast::MulticastGroup;

use super::{
    addr::{GroupIdSet, NetlinkProtocolId, NetlinkSocketAddr, PortNum, MAX_GROUPS},
    AnyMulticastMessage, AnyNetlinkSocket, AnyUnicastMessage,
};
use crate::{net::socket::netlink::addr::UNSPECIFIED_PORT, prelude::*, util::random::getrandom};

mod multicast;

pub(super) static NETLINK_SOCKET_TABLE: NetlinkSocketTable = NetlinkSocketTable::new();

/// All bound netlink sockets.
pub(super) struct NetlinkSocketTable {
    protocols: [RwMutex<Option<ProtocolSocketTable>>; MAX_ALLOWED_PROTOCOL_ID as usize],
}

impl NetlinkSocketTable {
    pub(super) const fn new() -> Self {
        Self {
            protocols: [const { RwMutex::new(None) }; MAX_ALLOWED_PROTOCOL_ID as usize],
        }
    }

    /// Adds a new netlink protocol.
    fn add_new_protocol(&self, protocol_id: NetlinkProtocolId) {
        if protocol_id >= MAX_ALLOWED_PROTOCOL_ID {
            return;
        }

        let mut protocol = self.protocols[protocol_id as usize].write();
        if protocol.is_some() {
            return;
        }

        let new_protocol = ProtocolSocketTable::new(protocol_id);
        *protocol = Some(new_protocol);
    }

    pub(super) fn bind(
        &self,
        protocol: NetlinkProtocolId,
        addr: &NetlinkSocketAddr,
        socket: AnyNetlinkSocket,
    ) -> Result<BoundHandle> {
        if protocol >= MAX_ALLOWED_PROTOCOL_ID {
            return_errno_with_message!(Errno::EINVAL, "the netlink protocol does not exist");
        }

        let mut protocol = self.protocols[protocol as usize].write();

        let Some(protocol_sockets) = protocol.as_mut() else {
            return_errno_with_message!(Errno::EINVAL, "the netlink protocol does not exist")
        };

        protocol_sockets.bind(addr, socket)
    }

    /// Sends a message to the specified socket that is bound to `dst_port`.
    pub(super) fn unicast(&self, dst_port: PortNum, message: AnyUnicastMessage) -> Result<()> {
        let protocol = message.protocol();

        // It's the caller's responsibility to ensure that the protocol is valid.
        let protocol = self.protocols[protocol as usize].read();
        let Some(protocol_sockets) = protocol.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the netlink protocol does not exist");
        };

        protocol_sockets.unicast(dst_port, message)
    }

    /// Mutlicasts a message to all sockets in the group specified by `message.src_addr()`.
    #[cfg_attr(not(ktest), expect(dead_code))]
    pub(super) fn multicast(&self, message: AnyMulticastMessage) -> Result<()> {
        let protocol = message.protocol();

        // It's the caller's responsibility to ensure that the protocol is valid.
        let protocol = self.protocols[protocol as usize].read();
        let Some(protocol_sockets) = protocol.as_ref() else {
            return_errno_with_message!(Errno::EINVAL, "the netlink protocol does not exist");
        };

        protocol_sockets.multicast(message)
    }
}

/// Bound socket table of a single netlink protocol.
///
/// Each table can have bound sockets for unicast
/// and at most 32 groups for multicast.
struct ProtocolSocketTable {
    id: NetlinkProtocolId,
    // TODO: This table should maintain the port number-to-socket relationship
    // to support both unicast and multicast effectively.
    unicast_sockets: BTreeMap<PortNum, AnyNetlinkSocket>,
    multicast_groups: Box<[MulticastGroup]>,
}

impl ProtocolSocketTable {
    /// Creates a new table.
    fn new(id: NetlinkProtocolId) -> Self {
        let multicast_groups = (0u32..MAX_GROUPS).map(|_| MulticastGroup::new()).collect();
        Self {
            id,
            unicast_sockets: BTreeMap::new(),
            multicast_groups,
        }
    }

    /// Binds a socket to the table.
    /// Returns the bound handle.
    ///
    /// The socket will be bound to a port specified by `addr.port()`.
    /// If `addr.port()` is zero, the kernel will assign a port,
    /// typically corresponding to the process ID of the current process.
    /// If the assigned port is already in use,
    /// this function will try to allocate a random unused port.
    ///
    /// Additionally, this socket can join one or more multicast groups,
    /// as specified in `addr.groups()`.
    fn bind(&mut self, addr: &NetlinkSocketAddr, socket: AnyNetlinkSocket) -> Result<BoundHandle> {
        let port = if addr.port() != UNSPECIFIED_PORT {
            addr.port()
        } else {
            let mut random_port = current!().pid();
            while random_port == UNSPECIFIED_PORT || self.unicast_sockets.contains_key(&random_port)
            {
                getrandom(random_port.as_bytes_mut()).unwrap();
            }
            random_port
        };

        if self.unicast_sockets.contains_key(&port) {
            return_errno_with_message!(Errno::EADDRINUSE, "the netlink port is already in use");
        }

        self.unicast_sockets.insert(port, socket);

        for group_id in addr.groups().ids_iter() {
            let group = &mut self.multicast_groups[group_id as usize];
            group.add_member(port);
        }

        Ok(BoundHandle::new(self.id, port, addr.groups()))
    }

    fn unicast(&self, dst_port: PortNum, message: AnyUnicastMessage) -> Result<()> {
        let Some(socket) = self.unicast_sockets.get(&dst_port) else {
            // FIXME: Should we return error here?
            return Ok(());
        };

        socket.enqueue_unicast_message(message)
    }

    fn multicast(&self, message: AnyMulticastMessage) -> Result<()> {
        let group_id = message.src_addr().groups();

        // FIXME: I guess a message can be sent to only one multiple group.
        // We need to further check the Linux behavior.
        assert_eq!(group_id.as_u32().count_ones(), 1);

        for group in group_id.ids_iter() {
            let Some(group) = self.multicast_groups.get(group as usize) else {
                continue;
            };

            for port_num in group.members() {
                let Some(socket) = self.unicast_sockets.get(port_num) else {
                    continue;
                };

                // FIXME: Should we slightly ignore the error if the socket's buffer has no enough space?
                socket.enqueue_mutlicast_message(message.clone())?;
            }
        }

        Ok(())
    }
}

/// A bound netlink socket address.
///
/// When dropping a `BoundHandle`,
/// the port will be automatically released.
#[derive(Debug)]
pub struct BoundHandle {
    protocol: NetlinkProtocolId,
    port: PortNum,
    groups: GroupIdSet,
}

impl BoundHandle {
    fn new(protocol: NetlinkProtocolId, port: PortNum, groups: GroupIdSet) -> Self {
        debug_assert_ne!(port, UNSPECIFIED_PORT);

        Self {
            protocol,
            port,
            groups,
        }
    }

    pub(super) const fn port(&self) -> PortNum {
        self.port
    }

    pub(super) const fn addr(&self) -> NetlinkSocketAddr {
        NetlinkSocketAddr::new(self.port, self.groups)
    }

    pub(super) fn add_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = NETLINK_SOCKET_TABLE.protocols[self.protocol as usize].write();

        let protocol_sockets = protocol_sockets.as_mut().unwrap();

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.add_member(self.port);
        }

        self.groups.add_groups(groups);
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = NETLINK_SOCKET_TABLE.protocols[self.protocol as usize].write();

        let protocol_sockets = protocol_sockets.as_mut().unwrap();

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }

        self.groups.drop_groups(groups);
    }

    pub(super) fn bind_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = NETLINK_SOCKET_TABLE.protocols[self.protocol as usize].write();

        let protocol_sockets = protocol_sockets.as_mut().unwrap();

        for group_id in self.groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.add_member(self.port);
        }

        self.groups = groups;
    }
}

impl Drop for BoundHandle {
    fn drop(&mut self) {
        let mut protocol_sockets = NETLINK_SOCKET_TABLE.protocols[self.protocol as usize].write();

        let Some(protocol_sockets) = protocol_sockets.as_mut() else {
            return;
        };

        protocol_sockets.unicast_sockets.remove(&self.port);

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

/// Returns whether the `protocol` is valid.
pub fn is_valid_protocol(protocol: NetlinkProtocolId) -> bool {
    protocol < MAX_ALLOWED_PROTOCOL_ID
}

/// Returns whether the `protocol` is reserved for system use.
fn is_standard_protocol(protocol: NetlinkProtocolId) -> bool {
    StandardNetlinkProtocol::try_from(protocol).is_ok()
}

/// Netlink protocols that are assigned for specific usage.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/netlink.h#L9>.
#[allow(non_camel_case_types)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum StandardNetlinkProtocol {
    /// Routing/device hook
    ROUTE = 0,
    /// Unused number
    UNUSED = 1,
    /// Reserved for user mode socket protocols
    USERSOCK = 2,
    /// Unused number, formerly ip_queue
    FIREWALL = 3,
    /// Socket monitoring
    SOCK_DIAG = 4,
    /// Netfilter/iptables ULOG
    NFLOG = 5,
    /// IPsec
    XFRM = 6,
    /// SELinux event notifications
    SELINUX = 7,
    /// Open-iSCSI
    ISCSI = 8,
    /// Auditing
    AUDIT = 9,
    FIB_LOOKUP = 10,
    CONNECTOR = 11,
    /// Netfilter subsystem
    NETFILTER = 12,
    IP6_FW = 13,
    /// DECnet routing messages
    DNRTMSG = 14,
    /// Kernel messages to userspace
    KOBJECT_UEVENT = 15,
    GENERIC = 16,
    /// Leave room for NETLINK_DM (DM Events)
    /// SCSI Transports
    SCSITRANSPORT = 18,
    ECRYPTFS = 19,
    RDMA = 20,
    /// Crypto layer
    CRYPTO = 21,
    /// SMC monitoring
    SMC = 22,
}

const MAX_ALLOWED_PROTOCOL_ID: NetlinkProtocolId = 32;
