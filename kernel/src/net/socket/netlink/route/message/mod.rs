// SPDX-License-Identifier: MPL-2.0

//! Netlink message types for the netlink route protocol.
//!
//! This module defines how to interpret messages sent from user space and how to write
//! kernel messages back to user space.

mod attr;
mod segment;

pub(super) use attr::{addr::AddrAttr, link::LinkAttr};
pub(super) use segment::{
    addr::{AddrMessageFlags, AddrSegment, AddrSegmentBody, RtScope},
    link::{LinkSegment, LinkSegmentBody},
    RtnlSegment,
};

use crate::net::socket::netlink::message::Message;

/// A netlink route message.
pub(in crate::net::socket::netlink) type RtnlMessage = Message<RtnlSegment>;
