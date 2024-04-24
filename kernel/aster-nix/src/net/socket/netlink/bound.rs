// SPDX-License-Identifier: MPL-2.0

use super::{
    addr::{FamilyId, NetlinkSocketAddr},
    family::NETLINK_FAMILIES,
    receiver::Receiver,
};
use crate::{
    events::IoEvents,
    net::socket::SendRecvFlags,
    prelude::*,
    process::signal::{CanPoll, Poller},
};

/// A bound netlink socket
pub struct BoundNetlink {
    family_id: FamilyId,
    addr: NetlinkSocketAddr,
    receiver: Receiver,
    remote: Mutex<Option<NetlinkSocketAddr>>,
}

impl BoundNetlink {
    pub fn new(family_id: FamilyId, addr: NetlinkSocketAddr, receiver: Receiver) -> Self {
        Self {
            family_id,
            addr,
            receiver,
            remote: Mutex::new(None),
        }
    }

    pub fn addr(&self) -> &NetlinkSocketAddr {
        &self.addr
    }

    pub fn set_remote(&self, remote: NetlinkSocketAddr) {
        *self.remote.lock() = Some(remote);
    }

    pub fn remote(&self) -> Option<NetlinkSocketAddr> {
        *self.remote.lock()
    }

    pub fn is_nonblocking(&self) -> bool {
        self.receiver.is_nonblocking()
    }

    pub fn recvfrom(
        &self,
        dst: &mut [u8],
        flags: SendRecvFlags,
    ) -> Result<(usize, NetlinkSocketAddr)> {
        let netlink_msg = self.receiver.receive()?;
        let len = netlink_msg.copy_to(dst);
        let addr = netlink_msg.src_addr();
        Ok((len, addr))
    }

    pub fn sendto(
        &self,
        remote: Option<NetlinkSocketAddr>,
        buf: &[u8],
        flags: SendRecvFlags,
    ) -> Result<usize> {
        let remote = if let Some(remote) = remote {
            remote
        } else if let Some(remote) = self.remote() {
            remote
        } else {
            return_errno_with_message!(
                Errno::EDESTADDRREQ,
                "the destination address is not specified"
            );
        };

        NETLINK_FAMILIES.send(self.family_id, &self.addr, buf, &remote)?;

        Ok(buf.len())
    }
}

impl CanPoll for BoundNetlink {
    fn poll_object(&self) -> &dyn CanPoll {
        &self.receiver
    }

    fn poll(&self, mut mask: IoEvents, poller: Option<&Poller>) -> IoEvents {
        let in_events = {
            mask.remove(IoEvents::OUT);
            self.receiver.poll(mask, poller)
        };
        // FIXME: how to correctly deal with OUT event?
        let out_events = IoEvents::OUT;
        in_events | out_events
    }
}
