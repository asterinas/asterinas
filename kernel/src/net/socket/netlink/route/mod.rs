// SPDX-License-Identifier: MPL-2.0

//! Netlink Route Socket.

pub(super) use message::RtnlMessage;

use crate::net::socket::netlink::{common::NetlinkSocket, table::NetlinkRouteProtocol};

mod bound;
mod kernel;
mod message;

pub type NetlinkRouteSocket = NetlinkSocket<NetlinkRouteProtocol>;
