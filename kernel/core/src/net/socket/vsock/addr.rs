// SPDX-License-Identifier: MPL-2.0

use crate::{net::socket::util::SocketAddr, prelude::*};

/// A vsock socket address.
#[repr(C)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct VsockSocketAddr {
    pub cid: u32,
    pub port: u32,
}

impl TryFrom<SocketAddr> for VsockSocketAddr {
    type Error = Error;

    fn try_from(value: SocketAddr) -> Result<Self> {
        let SocketAddr::Vsock(addr) = value else {
            return_errno_with_message!(Errno::EINVAL, "the socket address is not vsock");
        };

        Ok(addr)
    }
}

impl From<VsockSocketAddr> for SocketAddr {
    fn from(value: VsockSocketAddr) -> Self {
        SocketAddr::Vsock(value)
    }
}

pub(super) const VMADDR_CID_ANY: u32 = u32::MAX;
pub(super) const VMADDR_CID_HOST: u32 = 2;

pub(super) const VMADDR_PORT_ANY: u32 = u32::MAX;

pub(super) const UNSPECIFIED_VSOCK_ADDR: VsockSocketAddr = VsockSocketAddr {
    cid: VMADDR_CID_ANY,
    port: VMADDR_PORT_ANY,
};
