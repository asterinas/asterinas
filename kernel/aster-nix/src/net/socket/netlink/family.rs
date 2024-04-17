// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use super::{
    addr::{FamilyId, NetlinkSocketAddr, PortNum},
    multicast_group::{MuilicastGroup, MAX_GROUPS},
    sender::Sender,
};
use crate::prelude::*;

pub const NETLINK_FAMILIES: FamilySet = FamilySet::new();

/// All netlink families.
///
/// Some families are initialized by kernel for specific use.
/// While families are also allocated dynamically if user provides
/// a different family id.
///
/// TODO: should temporary family IDs be recycled?
pub struct FamilySet {
    families: RwMutex<BTreeMap<FamilyId, Mutex<NetlinkFamily>>>,
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
        let new_family = Mutex::new(NetlinkFamily::new(family_id));
        families.insert(family_id, new_family);
    }

    pub fn bind(&self, addr: &NetlinkSocketAddr, sender: Sender) -> Result<()> {
        let family_id = addr.family_id();
        if !is_valid_family_id(family_id) {
            return_errno_with_message!(Errno::EINVAL, "the socket address is invalid");
        }

        // Fast path: if the family already exists
        let families = self.families.upread();
        if let Some(family) = families.get(&family_id) {
            let mut family = family.lock();
            return family.bind(addr, sender);
        }

        // Add a new family, if the family does not exist
        let mut families = families.upgrade();
        if !families.contains_key(&family_id) {
            debug_assert!(is_temporary_family(family_id));
            let mut new_family = NetlinkFamily::new(family_id);
            new_family.bind(addr, sender)?;
            families.insert(family_id, Mutex::new(new_family));
        }

        Ok(())
    }

    pub fn send(&self, msg: &[u8], remote: NetlinkSocketAddr) -> Result<()> {
        todo!()
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
    /// this function will return `Errno::EADDRINUSE`.
    ///
    /// Meanwhile, this socket can join one or more multicast groups,
    /// which is `specified` in groups.
    pub fn bind(&mut self, addr: &NetlinkSocketAddr, sender: Sender) -> Result<()> {
        debug_assert_eq!(self.id, addr.family_id());
        debug_assert!(addr.port() != 0);

        let port = addr.port();

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
}

pub(super) fn init() {
    for family_id in NETLINK_ROUTE..=NETLINK_GENERIC {
        NETLINK_FAMILIES.add_new_family(family_id);
    }

    for family_id in NETLINK_SCSITRANSPORT..=NETLINK_SMC {
        NETLINK_FAMILIES.add_new_family(family_id);
    }
}

/// Returns whether the `family` corresponds to a temporary family,
/// that is to say, this family may be reclaimed
/// once all sockets of this family are dropped.
pub fn is_temporary_family(family_id: FamilyId) -> bool {
    family_id > NETLINK_SMC && family_id < MAX_LINK
}

/// Returns whether the `family` is a valid family id
pub fn is_valid_family_id(family_id: FamilyId) -> bool {
    family_id < MAX_LINK
}

/// Returns whether the `family` has reserved for some system use
pub fn is_system_family_id(family_id: FamilyId) -> bool {
    family_id <= NETLINK_SMC
}

// Below constants are from Linux:
// <https://elixir.bootlin.com/linux/v6.0.9/source/include/uapi/linux/netlink.h#L9>

/// Routing/device hook
pub const NETLINK_ROUTE: FamilyId = 0;
/// Unused number
pub const NETLINK_UNUSED: FamilyId = 1;
/// Reserved for user mode socket protocols
pub const NETLINK_USERSOCK: FamilyId = 2;
/// Unused number, formerly ip_queue
pub const NETLINK_FIREWALL: FamilyId = 3;
/// socket monitoring
pub const NETLINK_SOCK_DIAG: FamilyId = 4;
/// netfilter/iptables ULOG
pub const NETLINK_NFLOG: FamilyId = 5;
/// ipsec
pub const NETLINK_XFRM: FamilyId = 6;
/// SELinux event notifications
pub const NETLINK_SELINUX: FamilyId = 7;
/// Open-iSCSI
pub const NETLINK_ISCSI: FamilyId = 8;
/// auditing
pub const NETLINK_AUDIT: FamilyId = 9;
pub const NETLINK_FIB_LOOKUP: FamilyId = 10;
pub const NETLINK_CONNECTOR: FamilyId = 11;
/// netfilter subsystem
pub const NETLINK_NETFILTER: FamilyId = 12;
pub const NETLINK_IP6_FW: FamilyId = 13;
/// DECnet routing messages
pub const NETLINK_DNRTMSG: FamilyId = 14;
/// Kernel messages to userspace
pub const NETLINK_KOBJECT_UEVENT: FamilyId = 15;
pub const NETLINK_GENERIC: FamilyId = 16;
// leave room for NETLINK_DM (DM Events)
/// SCSI Transports
pub const NETLINK_SCSITRANSPORT: FamilyId = 18;
pub const NETLINK_ECRYPTFS: FamilyId = 19;
pub const NETLINK_RDMA: FamilyId = 20;
/// Crypto layer
pub const NETLINK_CRYPTO: FamilyId = 21;
/// SMC monitoring
pub const NETLINK_SMC: FamilyId = 22;

pub const MAX_LINK: FamilyId = 32;
