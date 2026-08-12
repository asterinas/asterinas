// SPDX-License-Identifier: MPL-2.0

use aster_bigtcp::{
    iface::{InterfaceName, InterfaceType},
    wire::{EthernetAddress, Ipv4Address},
};
use ostd::task::Task;

use crate::{
    net::{
        iface::{DEFAULT_TX_QUEUE_LEN, Iface, iter_all_ifaces},
        socket::Socket,
    },
    prelude::*,
    util::{
        ioctl::{RawIoctl, dispatch_ioctl},
        net::CSocketAddrFamily,
    },
};

mod ioctl_defs {
    use super::{CIfConf, CIfReq};
    use crate::util::ioctl::{InOutData, ioc};

    // Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/sockios.h#L56>.
    pub(super) type GetIfName       = ioc!(SIOCGIFNAME,     0x8910, InOutData<CIfReq>);
    pub(super) type GetIfConf       = ioc!(SIOCGIFCONF,     0x8912, InOutData<CIfConf>);
    pub(super) type GetIfFlags      = ioc!(SIOCGIFFLAGS,    0x8913, InOutData<CIfReq>);
    pub(super) type GetIfMetric     = ioc!(SIOCGIFMETRIC,   0x891D, InOutData<CIfReq>);
    pub(super) type GetIfMtu        = ioc!(SIOCGIFMTU,      0x8921, InOutData<CIfReq>);
    pub(super) type GetIfHwAddr     = ioc!(SIOCGIFHWADDR,   0x8927, InOutData<CIfReq>);
    pub(super) type GetIfIndex      = ioc!(SIOCGIFINDEX,    0x8933, InOutData<CIfReq>);
    pub(super) type GetIfTxQueueLen = ioc!(SIOCGIFTXQLEN,   0x8942, InOutData<CIfReq>);
}

pub(crate) fn socket_ioctl<T: Socket>(socket: &T, raw_ioctl: RawIoctl) -> Result<i32> {
    // Linux always handles `SIOCGIFCONF` first.
    match handle_get_ifconf(raw_ioctl) {
        Err(err) if err.error() == Errno::ENOTTY => (),
        res => return res,
    }

    // Each socket handles commands specific to its protocol or socket type.
    match socket.protocol_ioctl(raw_ioctl) {
        Err(err) if err.error() == Errno::ENOTTY => {}
        res => return res,
    }

    // Handle network device commands.
    network_device_ioctl(raw_ioctl)
}

fn handle_get_ifconf(raw_ioctl: RawIoctl) -> Result<i32> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        cmd @ GetIfConf => {
            let mut ifconf = cmd.read()?;
            ifconf.write_ifreqs()?;
            cmd.write(&ifconf)?;
            Ok(0)
        }
        _ => return_errno_with_message!(Errno::ENOTTY, "the socket ioctl command is unknown"),
    })
}

fn network_device_ioctl(raw_ioctl: RawIoctl) -> Result<i32> {
    use ioctl_defs::*;

    dispatch_ioctl!(match raw_ioctl {
        cmd @ GetIfName => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_index()?;
            ifreq.name = *iface.name();
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfFlags => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            ifreq.data = CIfReqData::new_flags(iface.flags().bits() as i16);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfMetric => {
            let mut ifreq = cmd.read()?;
            ifreq.get_iface_by_name()?;
            // Linux's per-interface metric is currently unused and always reported as zero.
            ifreq.data = CIfReqData::new_value(0);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfMtu => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            ifreq.data = CIfReqData::new_mtu(iface.mtu() as i32);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfHwAddr => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            let socket_addr = CSocketAddr::new_ethernet(iface.type_(), iface.ethernet_addr());
            ifreq.data = CIfReqData::new_hardware_addr(socket_addr);
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfIndex => {
            let mut ifreq = cmd.read()?;
            let iface = ifreq.get_iface_by_name()?;
            let index = iface.index();
            ifreq.data = CIfReqData::new_value(index.cast_signed());
            cmd.write(&ifreq)?;
            Ok(0)
        }
        cmd @ GetIfTxQueueLen => {
            let mut ifreq = cmd.read()?;
            ifreq.get_iface_by_name()?;
            ifreq.data = CIfReqData::new_value(DEFAULT_TX_QUEUE_LEN.cast_signed());
            cmd.write(&ifreq)?;
            Ok(0)
        }
        _ => return_errno_with_message!(Errno::ENOTTY, "the socket ioctl command is unknown"),
    })
}

/// `struct ifconf` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/if.h#L286>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct CIfConf {
    len: i32,
    data: Vaddr,
}

impl CIfConf {
    fn write_ifreqs(&mut self) -> Result<()> {
        let ifreqs = iter_all_ifaces().filter_map(|iface| {
            let address = {
                let ipv4_addr = iface.ipv4_cidr()?.address();
                CSocketAddr::new_ipv4(ipv4_addr)
            };

            Some(CIfReq {
                name: *iface.name(),
                data: CIfReqData::new_addr(address),
            })
        });

        if self.data == 0 {
            let count = ifreqs.count();
            self.len = (count * size_of::<CIfReq>()) as i32;
            return Ok(());
        }

        let task = Task::current().unwrap();
        let user_space = CurrentUserSpace::new(task.as_thread_local().unwrap());
        let mut writer = user_space.writer(self.data, self.len.max(0) as usize)?;

        let max_ifreqs = writer.avail() / size_of::<CIfReq>();
        let mut count = 0;
        for ifreq in ifreqs.take(max_ifreqs) {
            writer.write_val(&ifreq)?;
            count += 1;
        }
        self.len = (count * size_of::<CIfReq>()) as i32;

        Ok(())
    }
}

/// `struct ifreq` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/if.h#L234>.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
pub(in crate::net::socket) struct CIfReq {
    name: InterfaceName,
    data: CIfReqData,
}

/// The `ifr_ifru` union in `struct ifreq` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/if.h#L241>.
#[pod_union]
#[repr(C)]
#[derive(Clone, Copy)]
union CIfReqData {
    addr: CSocketAddr,
    dst_addr: CSocketAddr,
    broad_addr: CSocketAddr,
    netmask: CSocketAddr,
    hardware_addr: CSocketAddr,
    flags: i16,
    value: i32,
    mtu: i32,
    map: CIfMap,
    slave: InterfaceName,
    new_name: InterfaceName,
    data: Vaddr,
    settings: CIfSettings,
}

/// `struct sockaddr` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/linux/socket.h#L36>.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct CSocketAddr {
    family: u16,
    data: [u8; 14],
}

impl CSocketAddr {
    fn new_ipv4(ipv4_addr: Ipv4Address) -> Self {
        let family = CSocketAddrFamily::AF_INET as u16;

        let mut data = [0u8; 14];
        data[2..6].copy_from_slice(&ipv4_addr.octets());

        Self { family, data }
    }

    fn new_ethernet(iface_type: InterfaceType, ether_addr: Option<EthernetAddress>) -> Self {
        let family = iface_type as u16;

        let mut data = [0u8; 14];
        if let Some(ether_addr) = ether_addr {
            data[..6].copy_from_slice(&ether_addr.0)
        }

        Self { family, data }
    }
}

/// `struct if_settings` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/if.h#L207>.
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct CIfSettings {
    type_: u32,
    size: u32,
    data: Vaddr,
}

/// `struct ifmap` in Linux.
///
/// Reference: <https://elixir.bootlin.com/linux/v7.1/source/include/uapi/linux/if.h#L196>.
#[padding_struct]
#[repr(C)]
#[derive(Clone, Copy, Pod)]
struct CIfMap {
    mem_start: Vaddr,
    mem_end: Vaddr,
    base_addr: u16,
    irq: u8,
    dma: u8,
    port: u8,
}

impl CIfReq {
    pub(in crate::net::socket) fn set_sockaddr_ipv4(&mut self, ipv4_addr: Ipv4Address) {
        let socket_addr = CSocketAddr::new_ipv4(ipv4_addr);
        self.data = CIfReqData::new_addr(socket_addr);
    }

    pub(in crate::net::socket) fn get_iface_by_name(&mut self) -> Result<&'static Arc<Iface>> {
        iter_all_ifaces()
            .find(|iface| iface.name() == &self.name)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "no interface found"))
    }

    fn get_iface_by_index(&self) -> Result<&'static Arc<Iface>> {
        let index = *self.data.value();
        iter_all_ifaces()
            .find(|iface| iface.index().cast_signed() == index)
            .ok_or_else(|| Error::with_message(Errno::ENODEV, "no interface found"))
    }
}
