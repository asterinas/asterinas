// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use smoltcp::{
    iface::Config,
    phy::TxToken,
    wire::{self, Ipv4Cidr, Ipv4Packet},
};

use crate::{
    device::WithDevice,
    iface::{
        common::IfaceCommon, iface::internal::IfaceInternal, time::get_network_timestamp, Iface,
    },
};

pub struct IpIface<D, E> {
    driver: D,
    common: IfaceCommon<E>,
}

impl<D: WithDevice, E> IpIface<D, E> {
    pub fn new(driver: D, ip_cidr: Ipv4Cidr, ext: E) -> Arc<Self> {
        let interface = driver.with(|device| {
            let config = Config::new(smoltcp::wire::HardwareAddress::Ip);
            let now = get_network_timestamp();

            let mut interface = smoltcp::iface::Interface::new(config, device, now);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                ip_addrs.push(wire::IpCidr::Ipv4(ip_cidr)).unwrap();
            });
            interface
        });

        let common = IfaceCommon::new(interface, ext);

        Arc::new(Self { driver, common })
    }
}

impl<D, E> IfaceInternal<E> for IpIface<D, E> {
    fn common(&self) -> &IfaceCommon<E> {
        &self.common
    }
}

impl<D: WithDevice + 'static, E: Send + Sync> Iface<E> for IpIface<D, E> {
    fn raw_poll(&self, schedule_next_poll: &dyn Fn(Option<u64>)) {
        self.driver.with(|device| {
            let next_poll = self.common.poll(
                device,
                |data, _iface_cx, tx_token| Some((Ipv4Packet::new_checked(data).ok()?, tx_token)),
                |pkt, iface_cx, tx_token| {
                    let ip_repr = pkt.ip_repr();
                    tx_token.consume(ip_repr.buffer_len(), |buffer| {
                        ip_repr.emit(&mut buffer[..], &iface_cx.checksum_caps());
                        pkt.emit_payload(
                            &ip_repr,
                            &mut buffer[ip_repr.header_len()..],
                            &iface_cx.caps,
                        );
                    });
                },
            );
            schedule_next_poll(next_poll);
        });
    }
}
