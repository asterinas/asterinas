// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, sync::Arc};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::SpinLock;
use smoltcp::{
    iface::{Config, Context},
    wire::{
        self, ArpOperation, ArpRepr, EthernetAddress, EthernetProtocol, EthernetRepr, IpAddress,
        Ipv4Address, Ipv4AddressExt, Ipv4Cidr,
    },
};

use crate::{
    device::{AnyNetworkDevice, WithDevice, new_interface},
    ext::Ext,
    iface::{
        Iface, InterfaceFlags, InterfaceName, ScheduleNextPoll,
        common::{IfaceCommon, InterfaceType, PhyProcessResult, PollPhy, TxPacketWithDst},
        iface::internal::IfaceInternal,
        time::get_network_timestamp,
        wire::{arp, ether},
    },
    packet::{AllocatedTxPacket, LinkLayer, RxPacket, TransportLayer, TxPacket},
};

pub struct EtherIface<D, E: Ext> {
    driver: D,
    common: IfaceCommon<E>,
    ether_addr: EthernetAddress,
    arp_table: SpinLock<BTreeMap<Ipv4Address, EthernetAddress>, BottomHalfDisabled>,
}

impl<D: WithDevice, E: Ext> EtherIface<D, E> {
    pub fn new(
        driver: D,
        ether_addr: EthernetAddress,
        ip_cidr: Ipv4Cidr,
        gateway: Ipv4Address,
        name: InterfaceName,
        sched_poll: E::ScheduleNextPoll,
        flags: InterfaceFlags,
    ) -> Arc<Self> {
        let interface = driver.with(|device| {
            let config = Config::new(wire::HardwareAddress::Ethernet(ether_addr));
            let now = get_network_timestamp();

            let mut interface = new_interface(config, device.capabilities(), now);
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

        let common = IfaceCommon::new(name, InterfaceType::ETHER, flags, interface, sched_poll);

        Arc::new(Self {
            driver,
            common,
            ether_addr,
            arp_table: SpinLock::new(BTreeMap::new()),
        })
    }
}

impl<D, E: Ext> IfaceInternal<E> for EtherIface<D, E> {
    fn common(&self) -> &IfaceCommon<E> {
        &self.common
    }
}

impl<D: WithDevice + 'static, E: Ext> Iface<E> for EtherIface<D, E> {
    fn ethernet_addr(&self) -> Option<EthernetAddress> {
        Some(self.ether_addr)
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

impl<D: WithDevice + 'static, E: Ext> PollPhy for EtherIface<D, E> {
    fn process(
        &self,
        packet: RxPacket<LinkLayer>,
        iface_cx: &mut Context,
    ) -> Option<PhyProcessResult> {
        // Parse the Ethernet header. Ignore the packet if the header is ill-formed.
        let (packet, repr) = ether::parse(packet)?;

        // Ignore the Ethernet frame if it is not sent to us.
        if !repr.dst_addr.is_broadcast() && repr.dst_addr != self.ether_addr {
            return None;
        }

        // Ignore the Ethernet frame if the protocol is not supported.
        match repr.ethertype {
            EthernetProtocol::Ipv4 => Some(PhyProcessResult::Ipv4(packet)),
            EthernetProtocol::Ipv6 => Some(PhyProcessResult::Ipv6(packet)),
            EthernetProtocol::Arp => {
                let (_, arp_repr) = arp::parse(packet)?;
                let reply = self.process_arp(&arp_repr, iface_cx)?;
                match self.alloc_tx_buffer(0) {
                    Ok(buffer) => Self::emit_arp(&reply, buffer).map(PhyProcessResult::Tx),
                    Err(err) => {
                        ostd::error!("failed to allocate a network packet: {:?}", err);
                        None
                    }
                }
            }
            _ => None,
        }
    }

    fn dispatch(
        &self,
        packet: TxPacketWithDst,
        iface_cx: &mut Context,
    ) -> Option<TxPacket<LinkLayer>> {
        match self.resolve_ether_or_generate_arp(packet.dst_addr, iface_cx) {
            Ok(ether_repr) => Some(ether::emit(packet.packet, &ether_repr)),
            Err(Some(arp_repr)) => Self::emit_arp(&arp_repr, packet.packet.reset_to_allocated()),
            Err(None) => None,
        }
    }

    fn alloc_tx_buffer(&self, payload_len: usize) -> Result<AllocatedTxPacket, ostd::Error> {
        D::Device::alloc_tx_buffer(payload_len)
    }
}

impl<D, E: Ext> EtherIface<D, E> {
    fn process_arp(&self, arp_repr: &ArpRepr, iface_cx: &mut Context) -> Option<ArpRepr> {
        match arp_repr {
            ArpRepr::EthernetIpv4 {
                operation: ArpOperation::Reply,
                source_hardware_addr,
                source_protocol_addr,
                ..
            } => {
                // Ignore the ARP packet if the source addresses are not unicast or not local.
                if !source_hardware_addr.is_unicast()
                    || !iface_cx.in_same_network(&IpAddress::Ipv4(*source_protocol_addr))
                {
                    return None;
                }

                // Insert the mapping between the Ethernet address and the IP address.
                //
                // TODO: Remove the mapping if it expires.
                self.arp_table
                    .lock()
                    .insert(*source_protocol_addr, *source_hardware_addr);

                None
            }
            ArpRepr::EthernetIpv4 {
                operation: ArpOperation::Request,
                source_hardware_addr,
                source_protocol_addr,
                target_protocol_addr,
                ..
            } => {
                // Ignore the ARP packet if the source addresses are not unicast.
                if !source_hardware_addr.is_unicast() || !source_protocol_addr.x_is_unicast() {
                    return None;
                }

                // Ignore the ARP packet if we do not own the target address.
                if iface_cx
                    .ipv4_addr()
                    .is_none_or(|addr| addr != *target_protocol_addr)
                {
                    return None;
                }

                Some(ArpRepr::EthernetIpv4 {
                    operation: ArpOperation::Reply,
                    source_hardware_addr: self.ether_addr,
                    source_protocol_addr: *target_protocol_addr,
                    target_hardware_addr: *source_hardware_addr,
                    target_protocol_addr: *source_protocol_addr,
                })
            }
            _ => None,
        }
    }

    fn resolve_ether_or_generate_arp(
        &self,
        dst_addr: IpAddress,
        iface_cx: &mut Context,
    ) -> Result<EthernetRepr, Option<ArpRepr>> {
        // Resolve the next-hop IP address.
        let next_hop_ip = match iface_cx.route(&dst_addr, iface_cx.now()) {
            Some(IpAddress::Ipv4(next_hop_ip)) => next_hop_ip,
            Some(IpAddress::Ipv6(_)) => {
                // FIXME: Currently, we drop outbound IPv6 packets because neighbor discovery is not
                // implemented and we have no way to resolve the next-hop link-layer address.
                ostd::debug!("IPv6 neighbor discovery is not implemented for Ethernet interfaces");
                return Err(None);
            }
            None => return Err(None),
        };

        // Resolve the next-hop Ethernet address.
        let next_hop_ether = if next_hop_ip.is_broadcast() {
            EthernetAddress::BROADCAST
        } else if let Some(next_hop_ether) = self.arp_table.lock().get(&next_hop_ip) {
            *next_hop_ether
        } else {
            // If the next-hop Ethernet address cannot be resolved, we drop the original packet and
            // send an ARP packet instead. The upper layer should be responsible for detecting the
            // packet loss and retrying later to see if the Ethernet address is ready.
            return Err(Some(ArpRepr::EthernetIpv4 {
                operation: ArpOperation::Request,
                source_hardware_addr: self.ether_addr,
                source_protocol_addr: iface_cx.ipv4_addr().unwrap_or(Ipv4Address::UNSPECIFIED),
                target_hardware_addr: EthernetAddress::BROADCAST,
                target_protocol_addr: next_hop_ip,
            }));
        };

        Ok(EthernetRepr {
            src_addr: self.ether_addr,
            dst_addr: next_hop_ether,
            ethertype: EthernetProtocol::Ipv4,
        })
    }

    fn emit_arp(arp_repr: &ArpRepr, packet: AllocatedTxPacket) -> Option<TxPacket<LinkLayer>> {
        let ether_repr = match arp_repr {
            ArpRepr::EthernetIpv4 {
                source_hardware_addr,
                target_hardware_addr,
                ..
            } => EthernetRepr {
                src_addr: *source_hardware_addr,
                dst_addr: *target_hardware_addr,
                ethertype: EthernetProtocol::Arp,
            },
            _ => return None,
        };

        let packet = packet.to_builder_layer::<TransportLayer>().build();
        let packet = arp::emit(packet, arp_repr);
        Some(ether::emit(packet, &ether_repr))
    }
}
