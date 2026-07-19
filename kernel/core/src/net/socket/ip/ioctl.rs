// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{iface::InterfaceFlags, wire::Ipv4Address};

use crate::{
    net::socket::util::ioctl::CIfReq,
    prelude::*,
    util::ioctl::{RawIoctl, dispatch_ioctl},
};

mod ioctl_defs {
    use super::CIfReq;
    use crate::util::ioctl::{InOutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/sockios.h#L62>.
    pub(super) type GetIfAddr    = ioc!(SIOCGIFADDR,    0x8915, InOutData<CIfReq>);
    pub(super) type GetIfDstAddr = ioc!(SIOCGIFDSTADDR, 0x8917, InOutData<CIfReq>);
    pub(super) type GetIfBrdAddr = ioc!(SIOCGIFBRDADDR, 0x8919, InOutData<CIfReq>);
    pub(super) type GetIfNetmask = ioc!(SIOCGIFNETMASK, 0x891B, InOutData<CIfReq>);
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
        _ => return_errno_with_message!(Errno::ENOTTY, "the socket ioctl command is unknown"),
    })
}
