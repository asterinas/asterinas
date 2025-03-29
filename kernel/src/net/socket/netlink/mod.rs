// SPDX-License-Identifier: MPL-2.0

//! This module defines netlink socket.
//!
//! Netlink provides a standard socket-based user interface,
//! typically used for communication between user space and kernel space.
//! It can also be used for communication between two user processes.
//!
//! Each Netlink socket belongs to a Netlink protocol,
//! identified by a protocol ID (u32).
//! Protocols are usually defined based on specific functionality.
//! For example, the NETLINK_ROUTE protocol is used to retrieve or modify net devices settings.
//! Only sockets belonging to the same protocol can communicate with each other.
//! Some protocols are pre-defined by the kernel and have fixed purposes.
//! Users can also define their own custom protocol by providing a new protocol ID.
//!
//! Before communication,
//! a netlink socket needs to be bound to an address,
//! which consists of a port number and a multicast group number.
//!
//! The port number is used for unit cast communication,
//! while the multicast group number is used for multicast communication.
//!
//! For unicast communication, within each protocol,
//! each port number can only be bound to one socket.
//! However, different protocols can use the same port number.
//! Typically, the port number is the process ID of the current process.
//!
//! Multicast allows a message to be sent to one or multiple multicast groups at once.
//! Each protocol supports up to 32 multicast groups,
//! and each socket can belong to zero or multiple multicast groups.
//!
//! The communication in Netlink is similar to UDP,
//! as it does not require establishing a connection before sending messages.
//! The destination address needs to be specified when sending a message.
//!

mod addr;
mod multicast_group;
mod route;
mod table;

pub use addr::{NetlinkProtocolId, NetlinkSocketAddr};
pub use multicast_group::GroupIdSet;
pub use route::NetlinkRouteSocket;
pub use table::{is_valid_protocol, StandardNetlinkProtocol};

pub fn init() {
    table::init();
}
