// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::wire::{Ipv4Address, Ipv6Address, PortNum};

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
#[derive(Clone, Copy, Debug, Pod)]
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
        debug_assert_eq!(value.sin_family, CSocketAddrFamily::AF_INET as u16);
        (value.sin_addr.into(), value.sin_port.into())
    }
}

/// IPv4 4-byte address.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
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
#[derive(Clone, Copy, Debug, Pod)]
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

/// IPv6 socket address.
///
/// See <https://www.man7.org/linux/man-pages/man7/ipv6.7.html>.
///
/// This corresponds to `struct sockaddr_in6` in Linux.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
pub(super) struct CSocketAddrInet6 {
    /// Address family (AF_INET6).
    sin6_family: u16,
    /// Port number.
    sin6_port: CPortNum,
    /// Flow information.
    sin6_flowinfo: u32,
    /// IPv6 address.
    sin6_addr: CInet6Addr,
    /// Scope ID.
    sin6_scope_id: u32,
}

impl From<(Ipv6Address, PortNum)> for CSocketAddrInet6 {
    fn from(value: (Ipv6Address, PortNum)) -> Self {
        Self {
            sin6_family: CSocketAddrFamily::AF_INET6 as u16,
            sin6_port: value.1.into(),
            sin6_flowinfo: 0,
            sin6_addr: value.0.into(),
            sin6_scope_id: 0,
        }
    }
}

impl From<CSocketAddrInet6> for (Ipv6Address, PortNum) {
    fn from(value: CSocketAddrInet6) -> Self {
        debug_assert_eq!(value.sin6_family, CSocketAddrFamily::AF_INET6 as u16);
        (value.sin6_addr.into(), value.sin6_port.into())
    }
}

/// IPv6 16-byte address.
#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct CInet6Addr {
    s6_addr: [u8; 16],
}

impl From<Ipv6Address> for CInet6Addr {
    fn from(value: Ipv6Address) -> Self {
        let bits = value.to_bits();
        Self {
            s6_addr: bits.to_be_bytes(),
        }
    }
}

impl From<CInet6Addr> for Ipv6Address {
    fn from(value: CInet6Addr) -> Self {
        let bits = u128::from_be_bytes(value.s6_addr);
        Ipv6Address::from_bits(bits)
    }
}
