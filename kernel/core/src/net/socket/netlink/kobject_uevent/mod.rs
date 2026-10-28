// SPDX-License-Identifier: MPL-2.0

pub(super) use message::UeventMessage;
pub(crate) use message::broadcast_uevent;

use crate::net::socket::netlink::{common::NetlinkSocket, table::NetlinkUeventProtocol};

mod bound;
mod message;

pub(crate) type NetlinkUeventSocket = NetlinkSocket<NetlinkUeventProtocol>;
