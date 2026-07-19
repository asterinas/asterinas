// SPDX-License-Identifier: MPL-2.0

pub(super) use message::UeventMessage;

use crate::net::socket::netlink::{common::NetlinkSocket, table::NetlinkUeventProtocol};

mod bound;
mod message;

pub type NetlinkUeventSocket = NetlinkSocket<NetlinkUeventProtocol>;
