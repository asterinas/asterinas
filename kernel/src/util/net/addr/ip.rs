// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{Ipv4Address, PortNum};

use super::family::CSocketAddrFamily;
use crate::prelude::*;

/// IPv4 socket address.
///
/// See <https://www.man7.org/linux/man-pages/man7/ip.7.html>.
///
/// The pad bytes (namely `sin_zero`) do not appear in the man pages, but are actually required by
/// the Linux implementation. See
/// <https://elixir.bootlin.com/linux/v6.10.2/source/include/uapi/linux/in.h#L256>.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
pub(super) struct CSocketAddrInet {
    /// Address family (AF_INET).
    sin_family: u16,
    /// Port number.
    sin_port: CPortNum,
    /// IPv4 address.
    sin_addr: CInetAddr,
    /// Pad bytes to 16-byte `struct sockaddr`.
    sin_zero: [u8; 8],
}

impl From<(Ipv4Address, PortNum)> for CSocketAddrInet {
    fn from(value: (Ipv4Address, PortNum)) -> Self {
        Self {
            sin_family: CSocketAddrFamily::AF_INET as u16,
            sin_port: value.1.into(),
            sin_addr: value.0.into(),
            sin_zero: [0; 8],
        }
    }
}

impl From<CSocketAddrInet> for (Ipv4Address, PortNum) {
    fn from(value: CSocketAddrInet) -> Self {
        (value.sin_addr.into(), value.sin_port.into())
    }
}

/// IPv4 4-byte address.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct CInetAddr {
    s_addr: [u8; 4],
}

impl From<Ipv4Address> for CInetAddr {
    fn from(value: Ipv4Address) -> Self {
        Self {
            s_addr: value.octets(),
        }
    }
}

impl From<CInetAddr> for Ipv4Address {
    fn from(value: CInetAddr) -> Self {
        Self::from(value.s_addr)
    }
}

/// TCP/UDP port number.
#[repr(C)]
#[derive(Debug, Clone, Copy, Pod)]
struct CPortNum {
    port: [u8; 2],
}

impl From<PortNum> for CPortNum {
    fn from(value: PortNum) -> Self {
        Self {
            port: value.to_be_bytes(),
        }
    }
}

impl From<CPortNum> for PortNum {
    fn from(value: CPortNum) -> Self {
        Self::from_be_bytes(value.port)
    }
}
