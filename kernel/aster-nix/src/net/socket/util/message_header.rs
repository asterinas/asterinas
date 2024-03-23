// SPDX-License-Identifier: MPL-2.0

use super::socket_addr::SocketAddr;
use crate::{prelude::*, util::IoVecIter};

/// Message header used for sendmsg/recvmsg
#[derive(Debug)]
pub struct MessageHeader {
    pub(in crate::net) addr: Option<SocketAddr>,
    pub(in crate::net) io_vec_iter: IoVecIter,
    pub(in crate::net) control_message: Option<ControlMessgae>,
}

impl MessageHeader {
    pub fn new(
        addr: Option<SocketAddr>,
        io_vec_iter: IoVecIter,
        control_message: Option<ControlMessgae>,
    ) -> Self {
        Self {
            addr,
            io_vec_iter,
            control_message,
        }
    }

    pub fn addr(&self) -> Option<&SocketAddr> {
        self.addr.as_ref()
    }
}

/// Control message carried by MessageHeader.
///
/// TODO: Implement the struct. The struct is empty now.
#[derive(Debug)]
pub struct ControlMessgae;
