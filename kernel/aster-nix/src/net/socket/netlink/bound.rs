// SPDX-License-Identifier: MPL-2.0

use core::sync::atomic::AtomicBool;

use super::{addr::NetlinkSocketAddr, receiver::Receiver};
use crate::prelude::*;

/// A bound netlink socket
pub struct BoundNetlink {
    is_nonblocking: AtomicBool,
    local: Receiver,
    remote: Option<NetlinkSocketAddr>,
}

impl BoundNetlink {
    pub fn new(is_nonblocking: bool, addr: NetlinkSocketAddr) -> Self {
        todo!()
    }

    pub fn connect(&self, remote: NetlinkSocketAddr) -> Result<()> {
        todo!()
    }
}
