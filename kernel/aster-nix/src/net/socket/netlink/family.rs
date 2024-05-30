// SPDX-License-Identifier: MPL-2.0

use alloc::collections::{btree_map::Entry, BTreeMap};

use super::{
    addr::{FamilyId, NetlinkSocketAddr, PortNum},
    multicast_group::{GroupIdIter, MuilicastGroup, MAX_GROUPS},
    sender::Sender,
};
use crate::{
    net::socket::netlink::{addr::UNSPECIFIED_PORT, sender::NetlinkMessage},
    prelude::*,
};

pub static NETLINK_FAMILIES: FamilySet = FamilySet::new();

/// All netlink families.
///
/// Some families are initialized by kernel for specific use.
/// While families are also allocated dynamically if user provides
/// a different family id.
///
/// TODO: should temporary family IDs be recycled?
pub struct FamilySet {
    families: RwMutex<BTreeMap<FamilyId, RwMutex<NetlinkFamily>>>,
}

impl FamilySet {
    pub const fn new() -> Self {
        Self {
            families: RwMutex::new(BTreeMap::new()),
        }
    }

    /// Adds a new netlink family
    fn add_new_family(&self, family_id: FamilyId) {
        let mut families = self.families.write();
        if families.contains_key(&family_id) {
            return;
        }
        let new_family = RwMutex::new(NetlinkFamily::new(family_id));
        families.insert(family_id, new_family);
    }

    pub fn bind(
        &self,
        family_id: FamilyId,
        addr: &NetlinkSocketAddr,
        sender: Sender,
    ) -> Result<()> {
        if !is_valid_family_id(family_id) {
            return_errno_with_message!(Errno::EINVAL, "the socket address is invalid");
        }

        // Fast path: if the family already exists
        let families = self.families.upread();
        if let Some(family) = families.get(&family_id) {
            let mut family = family.write();
            return family.bind(addr, sender);
        }

        // Add a new family, if the family does not exist
        let mut families = families.upgrade();
        if let Entry::Vacant(e) = families.entry(family_id) {
            debug_assert!(is_temporary_family(family_id));
            let mut new_family = NetlinkFamily::new(family_id);
            new_family.bind(addr, sender)?;
            e.insert(RwMutex::new(new_family));
        }

        Ok(())
    }

    pub fn send(
        &self,
        family_id: FamilyId,
        src_addr: &NetlinkSocketAddr,
        msg: &[u8],
        remote: &NetlinkSocketAddr,
    ) -> Result<()> {
        let msg = NetlinkMessage::new(*src_addr, msg);

        let families = self.families.read();
        let family = {
            let family = families.get(&family_id).unwrap();
            family.read()
        };

        if remote.port() != UNSPECIFIED_PORT {
            if let Some(sender) = family.get_unicast_sender(remote.port()) {
                sender.send(msg.clone())?;
            }
        }

        let group_ids = remote.groups();
        for multicast_group in family.get_multicast_groups(group_ids.ids_iter()) {
            // FIXME: should we broadcast message to next group if some error occurs?
            // Currently, this won't be a problem since `broadcast` won't return errors.
            multicast_group.broadcast(msg.clone())?;
        }

        Ok(())
    }
}

/// A netlink family.
///
/// Each family has a unique `FamilyId`(u32).
/// Each family can have bound sockcts for unit cast
/// and at most 32 groups for multicast.
pub struct NetlinkFamily {
    id: FamilyId,
    unitcast_sockets: BTreeMap<PortNum, Sender>,
    multicast_groups: Box<[MuilicastGroup]>,
}

impl NetlinkFamily {
    /// Creates a new netlink family
    fn new(id: FamilyId) -> Self {
        let multicast_groups = (0u32..MAX_GROUPS)
            .map(|group_id| MuilicastGroup::new(id))
            .collect::<Vec<_>>();
        Self {
            id,
            unitcast_sockets: BTreeMap::new(),
            multicast_groups: multicast_groups.into_boxed_slice(),
        }
    }

    /// Binds a socket to the netlink family.
    /// Returns the bound addr.
    ///
    /// The socket will be bound to a port with `port_num`.
    /// If `port_num` is not provided, kernel will assign a port for it,
    /// typically, the port with the process id of current process.
    /// If the port is already used,
    /// this function will try to allocate a random unused port.
    ///
    /// Meanwhile, this socket can join one or more multicast groups,
    /// which is `specified` in groups.
    pub fn bind(&mut self, addr: &NetlinkSocketAddr, sender: Sender) -> Result<()> {
        let port = if addr.port() != UNSPECIFIED_PORT {
            addr.port()
        } else {
            let mut random_port = current!().pid();
            while random_port == UNSPECIFIED_PORT
                || self.unitcast_sockets.contains_key(&random_port)
            {
                getrandom::getrandom(random_port.as_bytes_mut()).unwrap();
            }
            random_port
        };

        if self.unitcast_sockets.contains_key(&port) {
            return_errno_with_message!(Errno::EADDRINUSE, "try to bind to an used port");
        }

        self.unitcast_sockets.insert(port, sender.clone());

        for group_id in addr.groups().ids_iter() {
            debug_assert!(group_id < MAX_GROUPS);
            let group = self.multicast_groups.get_mut(group_id as usize).unwrap();

            if group.contains_member(port) {
                group.remove_member(port);
            }

            group.add_member(port, sender.clone());
        }

        Ok(())
    }

    fn get_unicast_sender(&self, port: PortNum) -> Option<&Sender> {
        self.unitcast_sockets.get(&port)
    }

    fn get_multicast_groups<'a>(
        &'a self,
        iter: GroupIdIter<'a>,
    ) -> impl Iterator<Item = &'a MuilicastGroup> + '_ {
        iter.filter_map(|group_id| self.multicast_groups.get(group_id as usize))
    }
}

pub(super) fn init() {
    for family_id in 0..MAX_LINK {
        if is_standard_family_id(family_id) {
            NETLINK_FAMILIES.add_new_family(family_id);
        }
    }
}

/// Returns whether the `family` corresponds to a temporary family,
/// that is to say, this family may be reclaimed
/// once all sockets of this family are dropped.
pub fn is_temporary_family(family_id: FamilyId) -> bool {
    StandardNetlinkFamily::try_from(family_id).is_err() && family_id < MAX_LINK
}

/// Returns whether the `family` is a valid family id
pub fn is_valid_family_id(family_id: FamilyId) -> bool {
    family_id < MAX_LINK
}

/// Returns whether the `family` has reserved for some system use
pub fn is_standard_family_id(family_id: FamilyId) -> bool {
    StandardNetlinkFamily::try_from(family_id).is_ok()
}

#[derive(Debug, Clone, Copy)]
pub enum NetlinkFamilyType {
    Standard(StandardNetlinkFamily),
    Custom(FamilyId),
}

impl NetlinkFamilyType {
    pub fn family_id(&self) -> FamilyId {
        match self {
            NetlinkFamilyType::Standard(standard) => *standard as FamilyId,
            NetlinkFamilyType::Custom(family_id) => *family_id,
        }
    }
}

impl TryFrom<u32> for NetlinkFamilyType {
    type Error = Error;

    fn try_from(value: u32) -> Result<Self> {
        if let Ok(standard_family) = StandardNetlinkFamily::try_from(value) {
            return Ok(Self::Standard(standard_family));
        }

        if value < MAX_LINK {
            return Ok(Self::Custom(value));
        }

        return_errno_with_message!(Errno::EINVAL, "invalid netlink family id")
    }
}

/// These families are currently assigned for specific usage.
/// <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/netlink.h#L9>
#[allow(non_camel_case_types)]
#[repr(u32)]
#[derive(Debug, Clone, Copy, TryFromInt)]
pub enum StandardNetlinkFamily {
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

const MAX_LINK: FamilyId = 32;
