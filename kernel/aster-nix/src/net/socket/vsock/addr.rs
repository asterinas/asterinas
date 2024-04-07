// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::header::VsockAddr;

use crate::{net::socket::SocketAddr, prelude::*};

#[derive(Copy, Clone, Debug, Default, PartialEq, Eq, PartialOrd, Ord)]
pub struct VsockSocketAddr {
    pub cid: u32,
    pub port: u32,
}

impl VsockSocketAddr {
    pub fn new(cid: u32, port: u32) -> Self {
        Self { cid, port }
    }

    pub fn any_addr() -> Self {
        Self {
            cid: VMADDR_CID_ANY,
            port: VMADDR_PORT_ANY,
        }
    }
}

impl TryFrom<SocketAddr> for VsockSocketAddr {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        let (cid, port) = if let SocketAddr::Vsock(cid, port) = value {
            (cid, port)
        } else {
            return_errno_with_message!(Errno::EINVAL, "invalid vsock socket addr");
        };
        Ok(Self { cid, port })
    }
}

impl From<VsockSocketAddr> for SocketAddr {
    fn from(value: VsockSocketAddr) -> Self {
        SocketAddr::Vsock(value.cid, value.port)
    }
}

impl From<VsockAddr> for VsockSocketAddr {
    fn from(value: VsockAddr) -> Self {
        VsockSocketAddr {
            cid: value.cid as u32,
            port: value.port,
        }
    }
}

impl From<VsockSocketAddr> for VsockAddr {
    fn from(value: VsockSocketAddr) -> Self {
        VsockAddr {
            cid: value.cid as u64,
            port: value.port,
        }
    }
}

/// The vSocket equivalent of INADDR_ANY.
pub const VMADDR_CID_ANY: u32 = u32::MAX;
/// Use this as the destination CID in an address when referring to the local communication (loopback).
/// This was VMADDR_CID_RESERVED
pub const VMADDR_CID_LOCAL: u32 = 1;
/// Use this as the destination CID in an address when referring to the host (any process other than the hypervisor).
pub const VMADDR_CID_HOST: u32 = 2;
/// Bind to any available port.
pub const VMADDR_PORT_ANY: u32 = u32::MAX;
