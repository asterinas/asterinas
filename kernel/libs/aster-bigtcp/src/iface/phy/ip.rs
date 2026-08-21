// SPDX-License-Identifier: MPL-2.0

use alloc::sync::Arc;

use smoltcp::{
    iface::{Config, Context},
    wire::{self, EthernetAddress, Ipv4Cidr, Ipv6Cidr},
};

use crate::{
    device::{AnyNetworkDevice, WithDevice, new_interface},
    ext::Ext,
    iface::{
        Iface, InterfaceName, ScheduleNextPoll,
        common::{
            IfaceCommon, InterfaceFlags, InterfaceType, PhyProcessResult, PollPhy, TxPacketWithDst,
        },
        iface::internal::IfaceInternal,
        time::get_network_timestamp,
    },
    packet::{AllocatedTxPacket, LinkLayer, RxPacket, TxPacket},
};

pub struct IpIface<D, E: Ext> {
    driver: D,
    common: IfaceCommon<E>,
}

impl<D: WithDevice, E: Ext> IpIface<D, E> {
    // TODO: Support interfaces with multiple IPv4/IPv6 addresses or without IPv4 addresses.
    pub fn new(
        driver: D,
        ip_cidr: Ipv4Cidr,
        ipv6_cidr: Option<Ipv6Cidr>,
        name: InterfaceName,
        sched_poll: E::ScheduleNextPoll,
        type_: InterfaceType,
        flags: InterfaceFlags,
    ) -> Arc<Self> {
        let interface = driver.with(|device| {
            let config = Config::new(wire::HardwareAddress::Ip);
            let now = get_network_timestamp();

            let mut interface = new_interface(config, device.capabilities(), now);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                ip_addrs.push(wire::IpCidr::Ipv4(ip_cidr)).unwrap();
                if let Some(ipv6_cidr) = ipv6_cidr {
                    ip_addrs.push(wire::IpCidr::Ipv6(ipv6_cidr)).unwrap();
                }
            });
            interface
        });

        let common = IfaceCommon::new(name, type_, flags, interface, sched_poll);

        Arc::new(Self { driver, common })
    }
}

impl<D, E: Ext> IfaceInternal<E> for IpIface<D, E> {
    fn common(&self) -> &IfaceCommon<E> {
        &self.common
    }
}

impl<D: WithDevice + 'static, E: Ext> Iface<E> for IpIface<D, E> {
    fn ethernet_addr(&self) -> Option<EthernetAddress> {
        None
    }

    fn poll(&self) {
        self.driver.with(|device| {
            let next_poll = self.common.poll(device, self);
            device.notify_poll_end();
            self.common.sched_poll().schedule_next_poll(next_poll);
        });
    }

    fn mtu(&self) -> usize {
        self.driver
            .with(|device| device.capabilities().max_transmission_unit)
    }
}

impl<D: WithDevice + 'static, E: Ext> PollPhy for IpIface<D, E> {
    fn process(
        &self,
        packet: RxPacket<LinkLayer>,
        _iface_cx: &mut Context,
    ) -> Option<PhyProcessResult> {
        Some(PhyProcessResult::Ip(packet.peel(0)))
    }

    fn dispatch(
        &self,
        packet: TxPacketWithDst,
        _iface_cx: &mut Context,
    ) -> Option<TxPacket<LinkLayer>> {
        Some(packet.packet.pack(0))
    }

    fn alloc_tx_buffer(&self, payload_len: usize) -> Result<AllocatedTxPacket, ostd::Error> {
        D::Device::alloc_tx_buffer(payload_len)
    }
}
