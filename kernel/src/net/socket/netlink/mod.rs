// SPDX-License-Identifier: MPL-2.0

//! This module defines netlink sockets.
//!
//! Netlink provides a standardized, socket-based interface,
//! typically used for communication between user space and kernel space.
//! It can also be used for interaction between two user processes.
//!
//! Each netlink socket corresponds to
//! a netlink protocol identified by a protocol ID (u32).
//! Protocols are generally defined to serve specific functions.
//! For instance, the NETLINK_ROUTE protocol is employed
//! to retrieve or modify network device settings.
//! Only sockets associated with the same protocol can communicate with each other.
//! Some protocols are pre-defined by the kernel and serve fixed purposes,
//! but users can also establish custom protocols by specifying new protocol IDs.
//!
//! Before initiating communication,
//! a netlink socket must be bound to an address,
//! which consists of a port number and a multicast group number.
//!
//! The port number is used for unicast communication,
//! whereas the multicast group number is meant for multicast communication.
//!
//! In terms of unicast communication within each protocol,
//! a port number can only be bound to one socket.
//! However, the same port number can be utilized across different protocols.
//! Typically, the port number corresponds to the process ID of the running process.
//!
//! Multicast communication allows a message
//! to be sent to one or multiple multicast groups simultaneously.
//! Each protocol can support up to 32 multicast groups,
//! and a socket can belong to zero or multiple multicast groups.
//!
//! Netlink communication is akin to UDP in that
//! it does not require a connection to be established before sending messages.
//! The destination address must be specified when dispatching a message.
//!

mod addr;
mod common;
mod kobject_uevent;
mod message;
mod options;
mod receiver;
mod route;
mod table;

pub use addr::{GroupIdSet, NetlinkSocketAddr};
pub use kobject_uevent::NetlinkUeventSocket;
pub use options::{AddMembership, DropMembership};
pub use route::NetlinkRouteSocket;
pub use table::{is_valid_protocol, StandardNetlinkProtocol};

pub(in crate::net) fn init() {
    table::init();
}
