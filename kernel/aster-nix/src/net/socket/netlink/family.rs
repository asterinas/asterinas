// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;

use aster_util::slot_vec::SlotVec;

use super::{
    addr::{FamilyId, NetlinkSocketAddr, PortNum},
    multicast_group::MuilicastGroup,
    receiver::Receiver,
};
use crate::prelude::*;

/// All netlink families.
///
/// Some families are initialized by kernel for specific use.
/// While families are also allocated dynamically if user provides
/// a different family id.
///
/// TODO: should temporary family IDs be recycled?
pub struct FamilySet {
    families: Mutex<BTreeMap<FamilyId, NetlinkFamily>>,
}

impl FamilySet {
    pub const fn new() -> Self {
        Self {
            families: Mutex::new(BTreeMap::new()),
        }
    }

    pub fn bind(&self, addr: NetlinkSocketAddr, receiver: Receiver) -> Result<()> {
        todo!()
    }

    pub fn connect(&self, local: NetlinkSocketAddr, remote: NetlinkSocketAddr) -> Result<()> {
        todo!()
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
    unitcast_sockets: BTreeMap<PortNum, Receiver>,
    multicast_groups: SlotVec<Option<MuilicastGroup>>,
}
