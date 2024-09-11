// SPDX-License-Identifier: MPL-2.0

use super::socket_addr::SocketAddr;
use crate::prelude::*;

/// Message header used for sendmsg/recvmsg.
#[derive(Debug)]
pub struct MessageHeader {
    pub(in crate::net) addr: Option<SocketAddr>,
    pub(in crate::net) control_message: Option<ControlMessage>,
}

impl MessageHeader {
    /// Creates a new `MessageHeader`.
    pub const fn new(addr: Option<SocketAddr>, control_message: Option<ControlMessage>) -> Self {
        Self {
            addr,
            control_message,
        }
    }

    /// Returns the socket address.
    pub fn addr(&self) -> Option<&SocketAddr> {
        self.addr.as_ref()
    }
}

/// Control message carried by MessageHeader.
///
/// TODO: Implement the struct. The struct is empty now.
#[derive(Debug)]
pub struct ControlMessage;
