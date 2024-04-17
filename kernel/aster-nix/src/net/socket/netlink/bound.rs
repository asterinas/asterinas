// SPDX-License-Identifier: MPL-2.0

use super::{addr::NetlinkSocketAddr, receiver::Receiver};
use crate::prelude::*;

/// A bound netlink socket
pub struct BoundNetlink {
    addr: NetlinkSocketAddr,
    receiver: Receiver,
    remote: Option<NetlinkSocketAddr>,
}

impl BoundNetlink {
    pub fn new(addr: NetlinkSocketAddr, receiver: Receiver) -> Self {
        Self {
            addr,
            receiver,
            remote: None,
        }
    }

    pub fn addr(&self) -> &NetlinkSocketAddr {
        &self.addr
    }

    pub fn set_remote(&mut self, remote: NetlinkSocketAddr) {
        self.remote = Some(remote);
    }

    pub fn remote(&self) -> Option<&NetlinkSocketAddr> {
        self.remote.as_ref()
    }

    pub fn is_nonblocking(&self) -> bool {
        self.receiver.is_nonblocking()
    }

    pub fn recvfrom(&self, dst: &mut [u8]) -> Result<(usize, NetlinkSocketAddr)> {
        todo!()
    }
}
