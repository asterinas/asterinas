// SPDX-License-Identifier: MPL-2.0

//! Netlink kobject uevent socket.

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

const KOBJECT_UEVENT_GROUP_ID: u32 = 1;

pub(crate) type NetlinkUeventSocket = NetlinkSocket<NetlinkUeventProtocol>;

/// Broadcasts a serialized uevent payload.
pub(crate) fn broadcast_uevent(payload: Vec<u8>) {
    let groups = GroupIdSet::from_group_id(KOBJECT_UEVENT_GROUP_ID).unwrap();
    let src_addr = NetlinkSocketAddr::new(0, groups);
    let message = UeventMessage::new(payload, src_addr);
    let _ = NetlinkUeventProtocol::multicast(groups, message);
}
