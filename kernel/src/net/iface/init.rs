// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::ToOwned, sync::Arc};
use core::slice::Iter;

use aster_bigtcp::{
    device::WithDevice,
    iface::{InterfaceFlags, InterfaceType},
};
use aster_softirq::BottomHalfDisabled;
use spin::Once;

use super::{poll::poll_ifaces, Iface};
use crate::{net::iface::sched::PollScheduler, prelude::*};

static IFACES: Once<Vec<Arc<Iface>>> = Once::new();

pub fn loopback_iface() -> &'static Arc<Iface> {
    &IFACES.get().unwrap()[0]
}

pub fn virtio_iface() -> Option<&'static Arc<Iface>> {
    IFACES.get().unwrap().get(1)
}

pub fn iter_all_ifaces() -> Iter<'static, Arc<Iface>> {
    IFACES.get().unwrap().iter()
}

pub fn init() {
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

    if let Some(iface_virtio) = virtio_iface() {
        for (name, _) in aster_network::all_devices() {
            // TODO: further check that the irq num is the same as iface's irq num
            let callback = || iface_virtio.poll();
            aster_network::register_recv_callback(&name, callback);
            aster_network::register_send_callback(&name, callback);
        }
    }

    poll_ifaces();
}

fn new_virtio() -> Option<Arc<Iface>> {
    use aster_bigtcp::{
        iface::EtherIface,
        wire::{EthernetAddress, Ipv4Address, Ipv4Cidr},
    };
    use aster_network::AnyNetworkDevice;
    use aster_virtio::device::network::DEVICE_NAME;

    const VIRTIO_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
    const VIRTIO_ADDRESS_PREFIX_LEN: u8 = 24; // mask: 255.255.255.0
    const VIRTIO_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

    let virtio_net = aster_network::get_device(DEVICE_NAME)?;

    let ether_addr = virtio_net.lock().mac_addr().0;

    struct Wrapper(Arc<SpinLock<dyn AnyNetworkDevice, BottomHalfDisabled>>);

    impl WithDevice for Wrapper {
        type Device = dyn AnyNetworkDevice;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut Self::Device) -> R,
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

    Some(EtherIface::new(
        Wrapper(virtio_net),
        EthernetAddress(ether_addr),
        Ipv4Cidr::new(VIRTIO_ADDRESS, VIRTIO_ADDRESS_PREFIX_LEN),
        VIRTIO_GATEWAY,
        "eth0".to_owned(),
        PollScheduler::new(),
        flags,
    ))
}

fn new_loopback() -> Arc<Iface> {
    use aster_bigtcp::{
        device::{Loopback, Medium},
        iface::IpIface,
        wire::{Ipv4Address, Ipv4Cidr},
    };

    const LOOPBACK_ADDRESS: Ipv4Address = Ipv4Address::new(127, 0, 0, 1);
    const LOOPBACK_ADDRESS_PREFIX_LEN: u8 = 8; // mask: 255.0.0.0

    struct Wrapper(Mutex<Loopback>);

    impl WithDevice for Wrapper {
        type Device = Loopback;

        fn with<F, R>(&self, f: F) -> R
        where
            F: FnOnce(&mut Self::Device) -> R,
        {
            let mut device = self.0.lock();
            f(&mut device)
        }
    }

    // FIXME: These flags are currently hardcoded.
    // In the future, we should set appropriate values.
    let flags = InterfaceFlags::UP
        | InterfaceFlags::LOOPBACK
        | InterfaceFlags::RUNNING
        | InterfaceFlags::LOWER_UP;

    IpIface::new(
        Wrapper(Mutex::new(Loopback::new(Medium::Ip))),
        Ipv4Cidr::new(LOOPBACK_ADDRESS, LOOPBACK_ADDRESS_PREFIX_LEN),
        "lo".to_owned(),
        PollScheduler::new(),
        InterfaceType::LOOPBACK,
        flags,
    ) as Arc<Iface>
}
