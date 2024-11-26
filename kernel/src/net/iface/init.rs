// SPDX-License-Identifier: MPL-2.0

use alloc::{borrow::ToOwned, sync::Arc};

use aster_bigtcp::device::WithDevice;
use ostd::sync::LocalIrqDisabled;
use spin::Once;

use super::{poll::poll_ifaces, Iface};
use crate::{
    net::iface::ext::{IfaceEx, IfaceExt},
    prelude::*,
};

pub static IFACES: Once<Vec<Arc<Iface>>> = Once::new();

pub fn init() {
    IFACES.call_once(|| {
        let iface_virtio = new_virtio();
        let iface_loopback = new_loopback();
        vec![iface_virtio, iface_loopback]
    });

    for (name, _) in aster_network::all_devices() {
        let callback = || {
            // TODO: further check that the irq num is the same as iface's irq num
            let iface_virtio = &IFACES.get().unwrap()[0];
            iface_virtio.poll();
        };
        aster_network::register_recv_callback(&name, callback);
        aster_network::register_send_callback(&name, callback);
    }

    poll_ifaces();
}

fn new_virtio() -> Arc<Iface> {
    use aster_bigtcp::{
        iface::EtherIface,
        wire::{EthernetAddress, Ipv4Address, Ipv4Cidr},
    };
    use aster_network::AnyNetworkDevice;
    use aster_virtio::device::network::DEVICE_NAME;

    const VIRTIO_ADDRESS: Ipv4Address = Ipv4Address::new(10, 0, 2, 15);
    const VIRTIO_ADDRESS_PREFIX_LEN: u8 = 24; // mask: 255.255.255.0
    const VIRTIO_GATEWAY: Ipv4Address = Ipv4Address::new(10, 0, 2, 2);

    let virtio_net = aster_network::get_device(DEVICE_NAME).unwrap();

    let ether_addr = virtio_net.lock().mac_addr().0;

    struct Wrapper(Arc<SpinLock<dyn AnyNetworkDevice, LocalIrqDisabled>>);

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

    EtherIface::new(
        Wrapper(virtio_net),
        EthernetAddress(ether_addr),
        Ipv4Cidr::new(VIRTIO_ADDRESS, VIRTIO_ADDRESS_PREFIX_LEN),
        VIRTIO_GATEWAY,
        IfaceExt::new("virtio".to_owned()),
    )
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

    IpIface::new(
        Wrapper(Mutex::new(Loopback::new(Medium::Ip))),
        Ipv4Cidr::new(LOOPBACK_ADDRESS, LOOPBACK_ADDRESS_PREFIX_LEN),
        IfaceExt::new("lo".to_owned()),
    ) as _
}
