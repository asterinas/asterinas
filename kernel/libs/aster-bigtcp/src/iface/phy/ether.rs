// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use smoltcp::{
    iface::Config,
    wire::{self, EthernetAddress, Ipv4Address, Ipv4Cidr},
};

use crate::{
    device::WithDevice,
    iface::{
        common::IfaceCommon, iface::internal::IfaceInternal, time::get_network_timestamp, Iface,
    },
};

pub struct EtherIface<D: WithDevice, E> {
    driver: D,
    common: IfaceCommon<E>,
}

impl<D: WithDevice, E> EtherIface<D, E> {
    pub fn new(
        driver: D,
        ether_addr: EthernetAddress,
        ip_cidr: Ipv4Cidr,
        gateway: Ipv4Address,
        ext: E,
    ) -> Arc<Self> {
        let interface = driver.with(|device| {
            let config = Config::new(wire::HardwareAddress::Ethernet(ether_addr));
            let now = get_network_timestamp();

            let mut interface = smoltcp::iface::Interface::new(config, device, now);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                ip_addrs.push(wire::IpCidr::Ipv4(ip_cidr)).unwrap();
            });
            interface
                .routes_mut()
                .add_default_ipv4_route(gateway)
                .unwrap();
            interface
        });

        let common = IfaceCommon::new(interface, ext);

        Arc::new(Self { driver, common })
    }
}

impl<D: WithDevice, E> IfaceInternal<E> for EtherIface<D, E> {
    fn common(&self) -> &IfaceCommon<E> {
        &self.common
    }
}

impl<D: WithDevice, E: Send + Sync> Iface<E> for EtherIface<D, E> {
    fn raw_poll(&self, schedule_next_poll: &dyn Fn(Option<u64>)) {
        self.driver.with(|device| {
            let next_poll = self.common.poll(&mut *device);
            schedule_next_poll(next_poll);
        });
    }
}
