// SPDX-License-Identifier: MPL-2.0

use super::family::CSocketAddrFamily;
use crate::{net::socket::vsock::VsockSocketAddr, prelude::*};

/// VSOCK socket address.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct CSocketAddrVm {
    /// Address family (AF_VSOCK).
    svm_family: u16,
    /// Reserved (always zero).
    svm_reserved1: u16,
    /// Port number in host byte order.
    svm_port: u32,
    /// Address in host byte order.
    svm_cid: u32,
    /// Pad bytes to 16-byte `struct sockaddr` (always zero).
    svm_zero: [u8; 4],
}

impl From<VsockSocketAddr> for CSocketAddrVm {
    fn from(value: VsockSocketAddr) -> Self {
        Self {
            svm_family: CSocketAddrFamily::AF_VSOCK as u16,
            svm_reserved1: 0,
            svm_port: value.port,
            svm_cid: value.cid,
            svm_zero: [0; 4],
        }
    }
}

impl From<CSocketAddrVm> for VsockSocketAddr {
    fn from(value: CSocketAddrVm) -> Self {
        Self {
            cid: value.svm_cid,
            port: value.svm_port,
        }
    }
}
