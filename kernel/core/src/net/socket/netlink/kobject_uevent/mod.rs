// SPDX-License-Identifier: MPL-2.0

//! Netlink socket delivery for serialized device events.

pub(super) use message::UeventMessage;

use crate::{
    net::socket::netlink::{
        GroupIdSet, NetlinkSocketAddr,
        common::NetlinkSocket,
        table::{NetlinkUeventProtocol, SupportedNetlinkProtocol},
    },
    prelude::*,
};

mod bound;
mod message;
#[cfg(ktest)]
mod test;

const KOBJECT_UEVENT_GROUP_MASK: u32 = 1;

pub(crate) type NetlinkUeventSocket = NetlinkSocket<NetlinkUeventProtocol>;

/// Broadcasts a serialized uevent payload to group 1.
///
/// Delivery is best effort:
/// absent listeners and full receive queues drop the message,
/// and the sender is not told.
pub(crate) fn broadcast_uevent(payload: Vec<u8>) {
    let src_addr = NetlinkSocketAddr::new(0, GroupIdSet::new(KOBJECT_UEVENT_GROUP_MASK));
    let message = UeventMessage::new(payload, src_addr);
    let _ = NetlinkUeventProtocol::multicast(GroupIdSet::new(KOBJECT_UEVENT_GROUP_MASK), message);
}
