// SPDX-License-Identifier: MPL-2.0

use aster_virtio::device::socket::header::VsockDeviceAddr;

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
        let SocketAddr::Vsock(vsock_addr) = value else {
            return_errno_with_message!(Errno::EINVAL, "invalid vsock socket addr");
        };
        Ok(vsock_addr)
    }
}

impl From<VsockSocketAddr> for SocketAddr {
    fn from(value: VsockSocketAddr) -> Self {
        SocketAddr::Vsock(value)
    }
}

impl From<VsockDeviceAddr> for VsockSocketAddr {
    fn from(value: VsockDeviceAddr) -> Self {
        VsockSocketAddr {
            cid: value.cid as u32,
            port: value.port,
        }
    }
}

impl From<VsockSocketAddr> for VsockDeviceAddr {
    fn from(value: VsockSocketAddr) -> Self {
        VsockDeviceAddr {
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
