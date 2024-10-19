// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, sync::Arc};

use ostd::sync::{LocalIrqDisabled, SpinLock};
use smoltcp::{
    iface::{packet::Packet, Config, Context},
    phy::{DeviceCapabilities, TxToken},
    wire::{
        self, ArpOperation, ArpPacket, ArpRepr, EthernetAddress, EthernetFrame, EthernetProtocol,
        EthernetRepr, IpAddress, Ipv4Address, Ipv4AddressExt, Ipv4Cidr, Ipv4Packet,
    },
};

use crate::{
    device::WithDevice,
    iface::{
        common::IfaceCommon, iface::internal::IfaceInternal, time::get_network_timestamp, Iface,
    },
};

pub struct EtherIface<D, E> {
    driver: D,
    common: IfaceCommon<E>,
    ether_addr: EthernetAddress,
    arp_table: SpinLock<BTreeMap<Ipv4Address, EthernetAddress>, LocalIrqDisabled>,
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

        Arc::new(Self {
            driver,
            common,
            ether_addr,
            arp_table: SpinLock::new(BTreeMap::new()),
        })
    }
}

impl<D, E> IfaceInternal<E> for EtherIface<D, E> {
    fn common(&self) -> &IfaceCommon<E> {
        &self.common
    }
}

impl<D: WithDevice + 'static, E: Send + Sync> Iface<E> for EtherIface<D, E> {
    fn raw_poll(&self, schedule_next_poll: &dyn Fn(Option<u64>)) {
        self.driver.with(|device| {
            let next_poll = self.common.poll(
                &mut *device,
                |data, iface_cx, tx_token| self.process(data, iface_cx, tx_token),
                |pkt, iface_cx, tx_token| self.dispatch(pkt, iface_cx, tx_token),
            );
            schedule_next_poll(next_poll);
        });
    }
}

impl<D, E> EtherIface<D, E> {
    fn process<'pkt, T: TxToken>(
        &self,
        data: &'pkt [u8],
        iface_cx: &mut Context,
        tx_token: T,
    ) -> Option<(Ipv4Packet<&'pkt [u8]>, T)> {
        match self.parse_ip_or_process_arp(data, iface_cx) {
            Ok(pkt) => Some((pkt, tx_token)),
            Err(Some(arp)) => {
                Self::emit_arp(&arp, tx_token);
                None
            }
            Err(None) => None,
        }
    }

    fn parse_ip_or_process_arp<'pkt>(
        &self,
        data: &'pkt [u8],
        iface_cx: &mut Context,
    ) -> Result<Ipv4Packet<&'pkt [u8]>, Option<ArpRepr>> {
        // Parse the Ethernet header. Ignore the packet if the header is ill-formed.
        let frame = EthernetFrame::new_checked(data).map_err(|_| None)?;
        let repr = EthernetRepr::parse(&frame).map_err(|_| None)?;

        // Ignore the Ethernet frame if it is not sent to us.
        if !repr.dst_addr.is_broadcast() && repr.dst_addr != self.ether_addr {
            return Err(None);
        }

        // Ignore the Ethernet frame if the protocol is not supported.
        match repr.ethertype {
            EthernetProtocol::Ipv4 => {
                Ok(Ipv4Packet::new_checked(frame.payload()).map_err(|_| None)?)
            }
            EthernetProtocol::Arp => {
                let pkt = ArpPacket::new_checked(frame.payload()).map_err(|_| None)?;
                let arp = ArpRepr::parse(&pkt).map_err(|_| None)?;
                Err(self.process_arp(&arp, iface_cx))
            }
            _ => Err(None),
        }
    }

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

    fn dispatch<T: TxToken>(&self, pkt: &Packet, iface_cx: &mut Context, tx_token: T) {
        match self.resolve_ether_or_generate_arp(pkt, iface_cx) {
            Ok(ether) => Self::emit_ip(&ether, pkt, &iface_cx.caps, tx_token),
            Err(Some(arp)) => Self::emit_arp(&arp, tx_token),
            Err(None) => (),
        }
    }

    fn resolve_ether_or_generate_arp(
        &self,
        pkt: &Packet,
        iface_cx: &mut Context,
    ) -> Result<EthernetRepr, Option<ArpRepr>> {
        // Resolve the next-hop IP address.
        let next_hop_ip = match iface_cx.route(&pkt.ip_repr().dst_addr(), iface_cx.now()) {
            Some(IpAddress::Ipv4(next_hop_ip)) => next_hop_ip,
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

    /// Consumes the token and emits an IP packet.
    fn emit_ip<T: TxToken>(
        ether_repr: &EthernetRepr,
        ip_pkt: &Packet,
        caps: &DeviceCapabilities,
        tx_token: T,
    ) {
        tx_token.consume(
            ether_repr.buffer_len() + ip_pkt.ip_repr().buffer_len(),
            |buffer| {
                let mut frame = EthernetFrame::new_unchecked(buffer);
                ether_repr.emit(&mut frame);

                let ip_repr = ip_pkt.ip_repr();
                ip_repr.emit(frame.payload_mut(), &caps.checksum);
                ip_pkt.emit_payload(
                    &ip_repr,
                    &mut frame.payload_mut()[ip_repr.header_len()..],
                    caps,
                );
            },
        );
    }

    /// Consumes the token and emits an ARP packet.
    fn emit_arp<T: TxToken>(arp_repr: &ArpRepr, tx_token: T) {
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
            _ => return,
        };

        tx_token.consume(ether_repr.buffer_len() + arp_repr.buffer_len(), |buffer| {
            let mut frame = EthernetFrame::new_unchecked(buffer);
            ether_repr.emit(&mut frame);

            let mut pkt = ArpPacket::new_unchecked(frame.payload_mut());
            arp_repr.emit(&mut pkt);
        });
    }
}
