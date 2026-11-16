// SPDX-License-Identifier: MPL-2.0

use core::slice::Iter;

use aster_bigtcp::{
    device::{AnyNetworkDevice, WithDevice},
    iface::{InterfaceFlags, InterfaceName, InterfaceType},
};
use aster_softirq::BottomHalfDisabled;
use spin::Once;

use super::{Iface, poll::poll_ifaces};
use crate::{net::iface::sched::PollScheduler, prelude::*};

static IFACES: Once<Vec<Arc<Iface>>> = Once::new();

/// `ip=dhcp` on the kernel command line (the Linux `ip=` parameter): configure
/// the NIC with DHCP instead of the built-in `10.0.2.15/24` for QEMU.
static IP_PARAM: Once<String> = Once::new();
aster_cmdline::define_kv_param!("ip", IP_PARAM);

fn virtio_iface() -> Option<&'static Arc<Iface>> {
    IFACES.get().unwrap().get(1)
}

/// The network device driving `eth0`: virtio-net under a VMM, ENA on EC2.
fn primary_device_name() -> Option<&'static str> {
    [VIRTIO_DEVICE_NAME, ENA_DEVICE_NAME]
        .into_iter()
        .find(|name| aster_network::get_device(name).is_some())
}

pub(in crate::net) fn iter_all_ifaces() -> Iter<'static, Arc<Iface>> {
    IFACES.get().unwrap().iter()
}

// TODO: Support multiple network devices and avoid the hardcoded device name.
const VIRTIO_DEVICE_NAME: &str = aster_virtio::device::network::DEVICE_NAME;
const ENA_DEVICE_NAME: &str = aster_ena::DEVICE_NAME;

pub(in crate::net) fn init() {
    IFACES.call_once(|| {
        let mut ifaces = Vec::with_capacity(2);

        // Initialize loopback before virtio
        // to ensure the loopback interface index is ahead of virtio.
        ifaces.push(new_loopback());

        if let Some(iface_virtio) = new_virtio() {
            ifaces.push(iface_virtio);
        }

        ifaces
    });

    if let Some(iface_virtio) = virtio_iface()
        && let Some(name) = primary_device_name()
    {
        let callback = || iface_virtio.poll();
        aster_network::register_recv_callback(name, callback);
        aster_network::register_send_callback(name, callback);
    }

    poll_ifaces();
}

fn new_loopback() -> Arc<Iface> {
    use aster_bigtcp::{
        device::Loopback,
        iface::IpIface,
        wire::{Ipv4Address, Ipv4Cidr, Ipv6Address, Ipv6Cidr},
    };

    const LOOPBACK_ADDRESS: Ipv4Address = Ipv4Address::new(127, 0, 0, 1);
    const LOOPBACK_ADDRESS_PREFIX_LEN: u8 = 8; // mask: 255.0.0.0
    const LOOPBACK_IPV6_ADDRESS: Ipv6Address = Ipv6Address::new(0, 0, 0, 0, 0, 0, 0, 1);
    const LOOPBACK_IPV6_PREFIX_LEN: u8 = 128;

    struct Wrapper(Mutex<Loopback>);

    impl WithDevice for Wrapper {
        type Device = Loopback;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut dyn AnyNetworkDevice) -> R,
        {
            let mut device = self.0.lock();
            f(&mut *device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::LOOPBACK
        | InterfaceFlags::RUNNING
        | InterfaceFlags::LOWER_UP;

    IpIface::new(
        Wrapper(Mutex::new(Loopback::new())),
        Ipv4Cidr::new(LOOPBACK_ADDRESS, LOOPBACK_ADDRESS_PREFIX_LEN),
        Some(Ipv6Cidr::new(
            LOOPBACK_IPV6_ADDRESS,
            LOOPBACK_IPV6_PREFIX_LEN,
        )),
        InterfaceName::from_str_truncated("lo"),
        PollScheduler::new(),
        InterfaceType::LOOPBACK,
        flags,
    ) as Arc<Iface>
}

fn new_virtio() -> Option<Arc<Iface>> {
    use aster_bigtcp::{
        iface::EtherIface,
        wire::{Ipv4Address, Ipv4Cidr},
    };

    const VIRTIO_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
    const VIRTIO_ADDRESS_PREFIX_LEN: u8 = 24; // mask: 255.255.255.0
    const VIRTIO_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

    let device_name = primary_device_name()?;
    let virtio_net = aster_network::get_device(device_name)?;

    let ether_addr = virtio_net.lock().mac_addr();

    // `WithDevice::Device` selects the driver's static `alloc_tx_buffer`,
    // so each driver needs its own wrapper.
    struct Wrapper(Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>);

    impl WithDevice for Wrapper {
        type Device = aster_virtio::device::network::device::NetworkDevice;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut dyn AnyNetworkDevice) -> R,
        {
            let mut device = self.0.lock();
            f(&mut *device)
        }
    }

    struct EnaWrapper(Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>);

    impl WithDevice for EnaWrapper {
        type Device = aster_ena::EnaDevice;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut dyn AnyNetworkDevice) -> R,
        {
            let mut device = self.0.lock();
            f(&mut *device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::BROADCAST
        | InterfaceFlags::RUNNING
        | InterfaceFlags::MULTICAST
        | InterfaceFlags::LOWER_UP;

    let name = InterfaceName::from_str_truncated("eth0");
    let dhcp = IP_PARAM.get().is_some_and(|v| v == "dhcp");
    if dhcp {
        info!("eth0 ({}): configuring with DHCP (ip=dhcp)", device_name);
    }

    if device_name == ENA_DEVICE_NAME {
        return Some(if dhcp {
            EtherIface::new_dhcp(
                EnaWrapper(virtio_net),
                ether_addr,
                name,
                PollScheduler::new(),
                flags,
            ) as Arc<Iface>
        } else {
            EtherIface::new(
                EnaWrapper(virtio_net),
                ether_addr,
                Ipv4Cidr::new(VIRTIO_ADDRESS, VIRTIO_ADDRESS_PREFIX_LEN),
                VIRTIO_GATEWAY,
                name,
                PollScheduler::new(),
                flags,
            ) as Arc<Iface>
        });
    }

    Some(if dhcp {
        EtherIface::new_dhcp(
            Wrapper(virtio_net),
            ether_addr,
            name,
            PollScheduler::new(),
            flags,
        ) as Arc<Iface>
    } else {
        EtherIface::new(
            Wrapper(virtio_net),
            ether_addr,
            Ipv4Cidr::new(VIRTIO_ADDRESS, VIRTIO_ADDRESS_PREFIX_LEN),
            VIRTIO_GATEWAY,
            name,
            PollScheduler::new(),
            flags,
        ) as Arc<Iface>
    })
}
