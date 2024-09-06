// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use ostd::prelude::*;
use smoltcp::{
    iface::{Config, SocketHandle, SocketSet},
    socket::dhcpv4,
    wire::{self, EthernetAddress, IpCidr},
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
    dhcp_handle: SocketHandle,
}

impl<D: WithDevice, E> EtherIface<D, E> {
    pub fn new(driver: D, ether_addr: EthernetAddress, ext: E) -> Arc<Self> {
        let interface = driver.with(|device| {
            let ip_addr = IpCidr::new(wire::IpAddress::Ipv4(wire::Ipv4Address::UNSPECIFIED), 0);
            let config = Config::new(wire::HardwareAddress::Ethernet(ether_addr));
            let now = get_network_timestamp();

            let mut interface = smoltcp::iface::Interface::new(config, device, now);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                ip_addrs.push(ip_addr).unwrap();
            });
            interface
        });

        let common = IfaceCommon::new(interface, ext);

        let mut socket_set = common.sockets();
        let dhcp_handle = init_dhcp_client(&mut socket_set);
        drop(socket_set);

        Arc::new(Self {
            driver,
            common,
            dhcp_handle,
        })
    }

    /// FIXME: Once we have user program dhcp client, we may remove dhcp logic from kernel.
    pub fn process_dhcp(&self) {
        let mut socket_set = self.common.sockets();
        let dhcp_socket: &mut dhcpv4::Socket = socket_set.get_mut(self.dhcp_handle);

        let Some(dhcpv4::Event::Configured(config)) = dhcp_socket.poll() else {
            return;
        };
        let ip_addr = IpCidr::Ipv4(config.address);

        let mut interface = self.common.interface();

        println!("[DHCP] Local IP address: {:?}", ip_addr,);
        interface.update_ip_addrs(|ipaddrs| {
            if let Some(addr) = ipaddrs.iter_mut().next() {
                // already has ipaddrs
                *addr = ip_addr
            } else {
                // does not has ip addr
                ipaddrs.push(ip_addr).unwrap();
            }
        });

        if let Some(router) = config.router {
            println!("[DHCP] Router IP address: {:?}", router);
            interface
                .routes_mut()
                .add_default_ipv4_route(router)
                .unwrap();
        }
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

            self.process_dhcp();
        });
    }
}

/// Register a dhcp socket.
fn init_dhcp_client(socket_set: &mut SocketSet) -> SocketHandle {
    let dhcp_socket = dhcpv4::Socket::new();
    socket_set.add(dhcp_socket)
}
