// SPDX-License-Identifier: MPL-2.0

//! The general netlink message types for all netlink protocols

use crate::prelude::*;

#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub struct CNetlinkMessageHeader {
    /// Length of message including header
    pub(super) len: u32,
    /// Message content
    pub(super) type_: u16,
    /// Additional flags
    pub(super) flags: u16,
    /// Sequence number
    pub(super) seq: u32,
    /// Sending process port ID
    pub(super) pid: u32,
}

bitflags! {
    pub struct NetlinkMessageCommonFlags: u16 {
        /// It is request message.
        const REQUEST = 0x01;
        /// Multipart message, terminated by NLMSG_DONE
        const MULTI = 0x02;
        /// Reply with ack, with zero or error code
        const ACK = 0x04;
        /// Echo this request
        const ECHO = 0x08;
        /// Dump was inconsistent due to sequence change
        const DUMP_INTR = 0x10;
        /// Dump was filtered as requested
        const DUMP_FILTERED = 0x20;
    }
}

bitflags! {
    /// Modifiers to GET request
    pub struct GetRequestFlags: u16 {
        /// Specify tree root
        const ROOT = 0x100;
        /// Return all matching
        const MATCH = 0x200;
        /// Atomic get
        const ATOMIC = 0x400;
        const DUMP = Self::ROOT.bits | Self::MATCH.bits;
    }
}

bitflags! {
    /// Modifiers to NEW request
    pub struct NewRequestFlags: u16 {
        /// Override existing
        const REPLACE = 0x100;
        /// Do not touch, if it exists
        const EXCL = 0x200;
        /// Create, if it does not exist
        const CREATE = 0x400;
        /// Add to end of list
        const APPEND = 0x800;
    }
}

bitflags! {
    /// Modifiers to DELETE request
    pub struct DeleteRequestFlags: u16 {
        /// Do not delete recursively
        const NONREC = 0x100;
        /// Delete multiple objects
        const BULK = 0x200;
    }
}

bitflags! {
    /// Flags for ACK message
    pub struct AckFlags: u16 {
        const CAPPED = 0x100;
        const ACK_TLVS = 0x100;
    }
}
