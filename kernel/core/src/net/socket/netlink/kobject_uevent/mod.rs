// SPDX-License-Identifier: MPL-2.0

//! Netlink kobject uevent socket.

pub(super) use message::UeventMessage;

use crate::{
    net::socket::netlink::{
        GroupIdSet, NetlinkSocketAddr, addr, common::NetlinkSocket, table::NetlinkUeventProtocol,
    },
    prelude::*,
    process::{UserNamespace, credentials::capabilities::CapSet, posix_thread::AsPosixThread},
    security::lsm::hooks as lsm_hooks,
    thread::Thread,
};

mod bound;
mod message;

const KOBJECT_UEVENT_GROUP_ID: u32 = 1;

pub(crate) type NetlinkUeventSocket = NetlinkSocket<NetlinkUeventProtocol>;

/// Broadcasts a serialized uevent payload.
pub(crate) fn broadcast_uevent(payload: Vec<u8>) {
    let groups = GroupIdSet::from_group_id(KOBJECT_UEVENT_GROUP_ID).unwrap();
    let src_addr = NetlinkSocketAddr::new(addr::UNSPECIFIED_PORT, groups);
    let message = UeventMessage::new(payload, src_addr);
    NetlinkUeventProtocol::broadcast(groups, message);
}

/// Authorizes sending to `dst` (Linux omits `NL_CFG_F_NONROOT_SEND`).
///
/// Reference: <https://elixir.bootlin.com/linux/v6.13/source/lib/kobject_uevent.c#L783>.
pub(super) fn authorize_destination(dst: &NetlinkSocketAddr) -> Result<()> {
    if *dst == NetlinkSocketAddr::new_unspecified() {
        return Ok(());
    }

    // Kernel contexts have no POSIX credentials.
    let Some(thread) = Thread::current() else {
        return Ok(());
    };
    let Some(posix_thread) = thread.as_posix_thread() else {
        return Ok(());
    };

    lsm_hooks::on_capable(lsm_hooks::CapableContext::new(
        UserNamespace::get_init_singleton().as_ref(),
        posix_thread,
        CapSet::NET_ADMIN,
    ))
}
