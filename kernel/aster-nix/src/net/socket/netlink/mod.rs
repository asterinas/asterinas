// SPDX-License-Identifier: MPL-2.0

//! This module defines netlink socket.
//!
//! Netlink provides a standard socket-based user interface,
//! typically used for communication between user space and kernel space.
//! It can also be used for communication between two user processes.
//!
//! Each Netlink socket belongs to a Netlink family,
//! identified by a family ID (u32).
//! Families are usually defined based on specific functionality.
//! For example, the NETLINK_ROUTE family is used to retrieve or modify routing table entries.
//! Only sockets belonging to the same family can communicate with each other.
//! Some families are pre-defined by the kernel and have fixed purposes,
//! such as NETLINK_ROUTE.
//! Users can also define their own custom families by providing a new family ID.
//!
//! Before communication,
//! a netlink socket needs to be bound to an address,
//! which consists of a port number and a multicast group number.
//!
//! The port number is used for unit cast communication,
//! while the multicast group number is used for multicast communication.
//!
//! For unicast communication, within each family,
//! each port number can only be bound to one socket.
//! However, different families can use the same port number.
//! Typically, the port number is the PID (process ID) of the current process.
//!
//! Multicast allows a message to be sent to one or multiple multicast groups at once.
//! Each family supports up to 32 multicast groups,
//! and each socket can belong to zero or multiple multicast groups.
//!
//! The communication in Netlink is similar to UDP,
//! as it does not require establishing a connection before sending messages.
//! The destination address needs to be specified when sending a message.
//!

use aster_frame::sync::RwMutex;

use self::{bound::BoundNetlink, unbound::UnboundNetlink};

mod addr;
mod bound;
mod family;
mod multicast_group;
mod receiver;
mod sender;
mod unbound;

/// A netlink socket.
pub struct NetlinkSocket {
    inner: RwMutex<Inner>,
}

enum Inner {
    Unbound(UnboundNetlink),
    Bound(BoundNetlink),
}

impl NetlinkSocket {
    pub fn new(is_nonblocking: bool) -> Self {
        todo!()
    }
}
