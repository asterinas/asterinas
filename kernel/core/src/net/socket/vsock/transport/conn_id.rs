// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::header::VirtioVsockHdr;

use crate::net::socket::vsock::{VsockSocketAddr, transport::BoundPort};

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub(super) struct ConnId {
    pub(super) local_cid: u64,
    pub(super) peer_cid: u64,
    pub(super) local_port: u32,
    pub(super) peer_port: u32,
}

impl ConnId {
    pub(super) fn from_port_and_remote(port: &BoundPort, remote: VsockSocketAddr) -> Self {
        Self {
            local_cid: port.vsock_space().guest_cid(),
            peer_cid: remote.cid as u64,
            local_port: port.port(),
            peer_port: remote.port,
        }
    }

    pub(super) fn from_incoming_header(header: &VirtioVsockHdr) -> Self {
        Self {
            local_cid: header.dst_cid,
            peer_cid: header.src_cid,
            local_port: header.dst_port,
            peer_port: header.src_port,
        }
    }
}
