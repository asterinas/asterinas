// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    iface::InterfaceFlags,
    wire::{Ipv4Address, Ipv4Cidr},
};

use crate::{
    dispatch_ioctl,
    net::{route, socket::util::ioctl::CIfReq},
    prelude::*,
    util::ioctl::RawIoctl,
};

mod ioctl_defs {
    use super::{CIfReq, CRtEntry};
    use crate::{
        ioc,
        util::ioctl::{InData, InOutData},
    };

    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/sockios.h#L62>.
    pub(super) type GetIfAddr    = ioc!(SIOCGIFADDR,    0x8915, InOutData<CIfReq>);
    pub(super) type GetIfDstAddr = ioc!(SIOCGIFDSTADDR, 0x8917, InOutData<CIfReq>);
    pub(super) type GetIfBrdAddr = ioc!(SIOCGIFBRDADDR, 0x8919, InOutData<CIfReq>);
    pub(super) type GetIfNetmask = ioc!(SIOCGIFNETMASK, 0x891B, InOutData<CIfReq>);
    pub(super) type SetIfAddr    = ioc!(SIOCSIFADDR,    0x8916, InData<CIfReq>);
    pub(super) type SetIfBrdAddr = ioc!(SIOCSIFBRDADDR, 0x891A, InData<CIfReq>);
    pub(super) type SetIfNetmask = ioc!(SIOCSIFNETMASK, 0x891C, InData<CIfReq>);
    pub(super) type AddRoute     = ioc!(SIOCADDRT,      0x890B, InData<CRtEntry>);
    pub(super) type DelRoute     = ioc!(SIOCDELRT,      0x890C, InData<CRtEntry>);
}

/// `struct rtentry` in Linux (x86-64 layout, 120 bytes).
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/route.h#L30>.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(super) struct CRtEntry {
    pad1: u64,
    dst: [u8; 16],
    gateway: [u8; 16],
    genmask: [u8; 16],
    flags: u16,
    pad2: i16,
    pad2b: u32,
    pad3: u64,
    pad4: u64,
    metric: i16,
    pad5: [u8; 6],
    dev: u64,
    mtu: u64,
    window: u64,
    irtt: u16,
    pad6: [u8; 6],
}

const RTF_GATEWAY: u16 = 0x0002;

/// Extracts the IPv4 address of a `struct sockaddr_in` (AF_INET, port, addr).
fn sockaddr_ipv4(sa: &[u8; 16]) -> Option<Ipv4Address> {
    let family = u16::from_ne_bytes([sa[0], sa[1]]);
    if family != 2 /* AF_INET */ && family != 0 {
        return None;
    }
    Some(Ipv4Address::new(sa[4], sa[5], sa[6], sa[7]))
}

/// Applies a new address or netmask to `iface`, keeping the other half of the
/// CIDR, then rebuilds the routing tables.
fn set_iface_ipv4(
    ifreq: &mut CIfReq,
    addr: Option<Ipv4Address>,
    netmask: Option<Ipv4Address>,
) -> Result<i32> {
    let new = ifreq.get_sockaddr_ipv4()?;
    let iface = ifreq.get_iface_by_name()?;
    let cur = iface.ipv4_cidr();
    let address = addr
        .or(new.into())
        .unwrap_or_else(|| cur.map(|c| c.address()).unwrap_or(Ipv4Address::UNSPECIFIED));
    let (address, prefix) = match (addr, netmask) {
        (Some(a), _) => (a, cur.map(|c| c.prefix_len()).unwrap_or(24)),
        (None, Some(m)) => {
            let bits = u32::from_be_bytes(m.octets()).count_ones() as u8;
            (cur.map(|c| c.address()).unwrap_or(address), bits)
        }
        (None, None) => return Ok(0),
    };
    iface.set_ipv4_cidr(Ipv4Cidr::new(address, prefix));
    route::reload();
    Ok(0)
}

pub(super) fn ipv4_ioctl(raw_ioctl: RawIoctl) -> Result<i32> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        cmd @ GetIfAddr => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            let ipv4_addr = iface
                .ipv4_cidr()
                .ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, "no IPv4 address found"))?
                .address();
            ifreq.set_sockaddr_ipv4(ipv4_addr);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfDstAddr => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            // Asterinas does not yet support point-to-point interfaces,
            // so we report the local IPv4 address instead, consistent with Linux's behavior.
            let ipv4_addr = iface
                .ipv4_cidr()
                .ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, "no IPv4 address found"))?
                .address();
            ifreq.set_sockaddr_ipv4(ipv4_addr);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfBrdAddr => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            let broadcast_addr = if iface.flags().contains(InterfaceFlags::BROADCAST)
                && let Some(broadcast_addr) = iface.broadcast_addr()
            {
                broadcast_addr
            } else {
                Ipv4Address::UNSPECIFIED
            };
            ifreq.set_sockaddr_ipv4(broadcast_addr);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfNetmask => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            let netmask = iface
                .ipv4_cidr()
                .ok_or_else(|| Error::with_message(Errno::EADDRNOTAVAIL, "no IPv4 address found"))?
                .netmask();
            ifreq.set_sockaddr_ipv4(netmask);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ SetIfAddr => {
            let mut ifreq = cmd.read()?;
            let addr = ifreq.get_sockaddr_ipv4()?;
            set_iface_ipv4(&mut ifreq, Some(addr), None)
        }
        cmd @ SetIfNetmask => {
            let mut ifreq = cmd.read()?;
            let mask = ifreq.get_sockaddr_ipv4()?;
            set_iface_ipv4(&mut ifreq, None, Some(mask))
        }
        cmd @ SetIfBrdAddr => {
            // The broadcast address is derived from address and netmask; accept and ignore.
            let mut ifreq = cmd.read()?;
            let _ = ifreq.get_iface_by_name()?;
            Ok(0)
        }
        cmd @ AddRoute => {
            let rt = cmd.read()?;
            let dst = sockaddr_ipv4(&rt.dst).unwrap_or(Ipv4Address::UNSPECIFIED);
            let mask = sockaddr_ipv4(&rt.genmask).unwrap_or(Ipv4Address::UNSPECIFIED);
            if dst != Ipv4Address::UNSPECIFIED || mask != Ipv4Address::UNSPECIFIED {
                // Only the default route is configurable for now; connected routes come from the address.
                return_errno_with_message!(Errno::EINVAL, "only the default route can be added");
            }
            if rt.flags & RTF_GATEWAY == 0 {
                return_errno_with_message!(Errno::EINVAL, "default route needs a gateway");
            }
            let gateway = sockaddr_ipv4(&rt.gateway)
                .ok_or_else(|| Error::with_message(Errno::EINVAL, "bad gateway address"))?;
            // The route goes to the interface that owns the gateway's network, else the default NIC.
            let iface = crate::net::iface::iter_all_ifaces()
                .find(|i| i.ipv4_cidr().is_some_and(|c| c.contains_addr(&gateway)))
                .or_else(|| crate::net::iface::iter_all_ifaces().last())
                .ok_or_else(|| Error::with_message(Errno::ENETUNREACH, "no interface"))?;
            iface.set_ipv4_gateway(Some(gateway));
            route::reload();
            Ok(0)
        }
        cmd @ DelRoute => {
            let rt = cmd.read()?;
            let dst = sockaddr_ipv4(&rt.dst).unwrap_or(Ipv4Address::UNSPECIFIED);
            if dst != Ipv4Address::UNSPECIFIED {
                return_errno_with_message!(Errno::EINVAL, "only the default route can be deleted");
            }
            for iface in crate::net::iface::iter_all_ifaces() {
                iface.set_ipv4_gateway(None);
            }
            route::reload();
            Ok(0)
        }
        _ => return_errno_with_message!(Errno::ENOTTY, "the socket ioctl command is unknown"),
    })
}
