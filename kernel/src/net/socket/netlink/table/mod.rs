// SPDX-License-Identifier: MPL-2.0

use multicast::MulticastGroup;
pub(super) use multicast::MulticastMessage;
use spin::Once;

use super::addr::{GroupIdSet, NetlinkProtocolId, NetlinkSocketAddr, PortNum, MAX_GROUPS};
use crate::{
    net::socket::netlink::{
        addr::UNSPECIFIED_PORT, kobject_uevent::UeventMessage, receiver::MessageReceiver,
        route::RtnlMessage,
    },
    prelude::*,
    util::random::getrandom,
};

mod multicast;

static NETLINK_SOCKET_TABLE: Once<NetlinkSocketTable> = Once::new();

/// All bound netlink sockets.
struct NetlinkSocketTable {
    route: RwMutex<ProtocolSocketTable<RtnlMessage>>,
    uevent: RwMutex<ProtocolSocketTable<UeventMessage>>,
}

impl NetlinkSocketTable {
    fn new() -> Self {
        Self {
            route: RwMutex::new(ProtocolSocketTable::new()),
            uevent: RwMutex::new(ProtocolSocketTable::new()),
        }
    }
}

pub trait SupportedNetlinkProtocol {
    type Message: 'static + Send;

    fn socket_table() -> &'static RwMutex<ProtocolSocketTable<Self::Message>>;

    fn bind(
        addr: &NetlinkSocketAddr,
        receiver: MessageReceiver<Self::Message>,
    ) -> Result<BoundHandle<Self::Message>> {
        let mut socket_table = Self::socket_table().write();
        socket_table.bind(Self::socket_table(), addr, receiver)
    }

    fn unicast(dst_port: PortNum, message: Self::Message) -> Result<()> {
        let socket_table = Self::socket_table().read();
        socket_table.unicast(dst_port, message)
    }

    fn multicast(dst_groups: GroupIdSet, message: Self::Message) -> Result<()>
    where
        Self::Message: MulticastMessage,
    {
        let socket_table = Self::socket_table().read();
        socket_table.multicast(dst_groups, message)
    }
}

pub enum NetlinkRouteProtocol {}

impl SupportedNetlinkProtocol for NetlinkRouteProtocol {
    type Message = RtnlMessage;

    fn socket_table() -> &'static RwMutex<ProtocolSocketTable<Self::Message>> {
        &NETLINK_SOCKET_TABLE.get().unwrap().route
    }
}

pub enum NetlinkUeventProtocol {}

impl SupportedNetlinkProtocol for NetlinkUeventProtocol {
    type Message = UeventMessage;

    fn socket_table() -> &'static RwMutex<ProtocolSocketTable<Self::Message>> {
        &NETLINK_SOCKET_TABLE.get().unwrap().uevent
    }
}

/// Bound socket table of a single netlink protocol.
///
/// Each table can have bound sockets for unicast
/// and at most 32 groups for multicast.
pub struct ProtocolSocketTable<Message> {
    unicast_sockets: BTreeMap<PortNum, MessageReceiver<Message>>,
    multicast_groups: Box<[MulticastGroup]>,
}

impl<Message: 'static> ProtocolSocketTable<Message> {
    /// Creates a new table.
    fn new() -> Self {
        let multicast_groups = (0u32..MAX_GROUPS).map(|_| MulticastGroup::new()).collect();
        Self {
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
    fn bind(
        &mut self,
        socket_table: &'static RwMutex<ProtocolSocketTable<Message>>,
        addr: &NetlinkSocketAddr,
        receiver: MessageReceiver<Message>,
    ) -> Result<BoundHandle<Message>> {
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

        self.unicast_sockets.insert(port, receiver);

        for group_id in addr.groups().ids_iter() {
            let group = &mut self.multicast_groups[group_id as usize];
            group.add_member(port);
        }

        Ok(BoundHandle::new(socket_table, port, addr.groups()))
    }

    fn unicast(&self, dst_port: PortNum, message: Message) -> Result<()> {
        let Some(receiver) = self.unicast_sockets.get(&dst_port) else {
            // FIXME: Should we return error here?
            return Ok(());
        };

        receiver.enqueue_message(message)
    }

    fn multicast(&self, dst_groups: GroupIdSet, message: Message) -> Result<()>
    where
        Message: MulticastMessage,
    {
        for group in dst_groups.ids_iter() {
            let Some(group) = self.multicast_groups.get(group as usize) else {
                continue;
            };

            for port_num in group.members() {
                let Some(receiver) = self.unicast_sockets.get(port_num) else {
                    continue;
                };

                // FIXME: Should we slightly ignore the error if the socket's buffer has no enough space?
                receiver.enqueue_message(message.clone())?;
            }
        }

        Ok(())
    }
}

/// A bound netlink socket address.
///
/// When dropping a `BoundHandle`,
/// the port will be automatically released.
pub struct BoundHandle<Message: 'static> {
    socket_table: &'static RwMutex<ProtocolSocketTable<Message>>,
    port: PortNum,
    groups: GroupIdSet,
}

impl<Message: 'static> BoundHandle<Message> {
    fn new(
        socket_table: &'static RwMutex<ProtocolSocketTable<Message>>,
        port: PortNum,
        groups: GroupIdSet,
    ) -> Self {
        debug_assert_ne!(port, UNSPECIFIED_PORT);

        Self {
            socket_table,
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
        let mut protocol_sockets = self.socket_table.write();

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.add_member(self.port);
        }

        self.groups.add_groups(groups);
    }

    pub(super) fn drop_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = self.socket_table.write();

        for group_id in groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }

        self.groups.drop_groups(groups);
    }

    pub(super) fn bind_groups(&mut self, groups: GroupIdSet) {
        let mut protocol_sockets = self.socket_table.write();

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

impl<Message: 'static> Drop for BoundHandle<Message> {
    fn drop(&mut self) {
        let mut protocol_sockets = self.socket_table.write();

        protocol_sockets.unicast_sockets.remove(&self.port);

        for group_id in self.groups.ids_iter() {
            let group = &mut protocol_sockets.multicast_groups[group_id as usize];
            group.remove_member(self.port);
        }
    }
}

pub(super) fn init() {
    NETLINK_SOCKET_TABLE.call_once(NetlinkSocketTable::new);
}

/// Returns whether the `protocol` is valid.
pub fn is_valid_protocol(protocol: NetlinkProtocolId) -> bool {
    protocol < MAX_ALLOWED_PROTOCOL_ID
}

/// Netlink protocols that are assigned for specific usage.
///
/// Reference: <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/netlink.h#L9>.
#[expect(non_camel_case_types)]
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
