// SPDX-License-Identifier: MPL-2.0

use alloc::{collections::btree_map::BTreeMap, ffi::CString, sync::Arc, vec, vec::Vec};

use aster_softirq::BottomHalfDisabled;
use ostd::sync::SpinLock;
use smoltcp::{
    iface::{Config, Context, packet::Packet},
    phy::{Device, DeviceCapabilities, TxToken},
    wire::{
        self, ArpOperation, ArpPacket, ArpRepr, EthernetAddress, EthernetFrame, EthernetProtocol,
        EthernetRepr, HardwareAddress, Icmpv6Packet, Icmpv6Repr, IpAddress, IpProtocol,
        Ipv4Address, Ipv4AddressExt, Ipv4Cidr, Ipv4Packet, Ipv6Address, Ipv6AddressExt, Ipv6Cidr,
        Ipv6Packet, Ipv6Repr, NdiscNeighborFlags, NdiscRepr, RawHardwareAddress,
    },
};

use crate::{
    device::{NotifyDevice, WithDevice},
    ext::Ext,
    iface::{
        Iface, InterfaceFlags, ScheduleNextPoll,
        common::{IfaceCommon, InterfaceType, IpPacket},
        iface::internal::IfaceInternal,
        time::get_network_timestamp,
    },
};

pub struct EtherIface<D, E: Ext> {
    driver: D,
    common: IfaceCommon<E>,
    ether_addr: EthernetAddress,
    arp_table: SpinLock<BTreeMap<Ipv4Address, EthernetAddress>, BottomHalfDisabled>,
    ndisc_table: SpinLock<BTreeMap<Ipv6Address, EthernetAddress>, BottomHalfDisabled>,
    pending_ipv6_packets: SpinLock<BTreeMap<Ipv6Address, PendingIpv6Packet>, BottomHalfDisabled>,
}

const MAX_PENDING_IPV6_PACKETS: usize = 64;
const PENDING_IPV6_PACKET_TIMEOUT_MS: i64 = 1_000;

struct PendingIpv6Packet {
    packet: Vec<u8>,
    expires_at_ms: i64,
}

#[derive(Clone, Copy, Debug)]
pub struct EtherIpConfig {
    ipv4_cidr: Ipv4Cidr,
    ipv6_cidr: Option<Ipv6Cidr>,
    ipv4_gateway: Ipv4Address,
    ipv6_gateway: Option<Ipv6Address>,
}

impl EtherIpConfig {
    pub fn new(
        ipv4_cidr: Ipv4Cidr,
        ipv6_cidr: Option<Ipv6Cidr>,
        ipv4_gateway: Ipv4Address,
        ipv6_gateway: Option<Ipv6Address>,
    ) -> Self {
        Self {
            ipv4_cidr,
            ipv6_cidr,
            ipv4_gateway,
            ipv6_gateway,
        }
    }
}

enum EtherIngress<'pkt> {
    Ip(IpPacket<'pkt>),
    Arp(ArpRepr),
    Icmpv6(Icmpv6Message<'static>),
    PendingIpv6(EthernetAddress, Vec<u8>),
    Drop,
}

enum EtherTx {
    Ip(EthernetRepr),
    Arp(ArpRepr),
    Icmpv6(Icmpv6Message<'static>),
    Drop,
}

struct Icmpv6Message<'a> {
    src_ip: Ipv6Address,
    dst_ip: Ipv6Address,
    dst_ether: EthernetAddress,
    repr: Icmpv6Repr<'a>,
}

impl<D: WithDevice, E: Ext> EtherIface<D, E> {
    pub fn new(
        driver: D,
        ether_addr: EthernetAddress,
        ip_config: EtherIpConfig,
        name: CString,
        sched_poll: E::ScheduleNextPoll,
        flags: InterfaceFlags,
    ) -> Arc<Self> {
        let interface = driver.with(|device| {
            let config = Config::new(HardwareAddress::Ethernet(ether_addr));
            let now = get_network_timestamp();

            let mut interface = smoltcp::iface::Interface::new(config, device, now);
            interface.update_ip_addrs(|ip_addrs| {
                debug_assert!(ip_addrs.is_empty());
                ip_addrs
                    .push(wire::IpCidr::Ipv4(ip_config.ipv4_cidr))
                    .unwrap();
                if let Some(ipv6_cidr) = ip_config.ipv6_cidr {
                    ip_addrs.push(wire::IpCidr::Ipv6(ipv6_cidr)).unwrap();
                }
            });
            interface
                .routes_mut()
                .add_default_ipv4_route(ip_config.ipv4_gateway)
                .unwrap();
            if let Some(ipv6_gateway) = ip_config.ipv6_gateway {
                interface
                    .routes_mut()
                    .add_default_ipv6_route(ipv6_gateway)
                    .unwrap();
            }
            interface
        });

        let common = IfaceCommon::new(name, InterfaceType::ETHER, flags, interface, sched_poll);

        Arc::new(Self {
            driver,
            common,
            ether_addr,
            arp_table: SpinLock::new(BTreeMap::new()),
            ndisc_table: SpinLock::new(BTreeMap::new()),
            pending_ipv6_packets: SpinLock::new(BTreeMap::new()),
        })
    }
}

impl<D, E: Ext> IfaceInternal<E> for EtherIface<D, E> {
    fn common(&self) -> &IfaceCommon<E> {
        &self.common
    }
}

impl<D: WithDevice + 'static, E: Ext> Iface<E> for EtherIface<D, E>
where
    D::Device: NotifyDevice,
{
    fn poll(&self) {
        self.driver.with(|device| {
            let next_poll = self.common.poll(
                &mut *device,
                |data, iface_cx, tx_token| self.process(data, iface_cx, tx_token),
                |pkt, iface_cx, tx_token| self.dispatch(pkt, iface_cx, tx_token),
            );
            device.notify_poll_end();
            self.common.sched_poll().schedule_next_poll(next_poll);
        });
    }

    fn mtu(&self) -> usize {
        self.driver
            .with(|device| device.capabilities().max_transmission_unit)
    }
}

impl<D, E: Ext> EtherIface<D, E> {
    fn process<'pkt, T: TxToken>(
        &self,
        data: &'pkt [u8],
        iface_cx: &mut Context,
        tx_token: T,
    ) -> Option<(IpPacket<'pkt>, T)> {
        match self.parse_ip_or_process_neighbor(data, iface_cx) {
            EtherIngress::Ip(pkt) => Some((pkt, tx_token)),
            EtherIngress::Arp(arp) => {
                Self::emit_arp(&arp, tx_token);
                None
            }
            EtherIngress::Icmpv6(message) => {
                self.emit_icmpv6(&message, iface_cx, tx_token);
                None
            }
            EtherIngress::PendingIpv6(dst_ether, packet) => {
                Self::emit_ipv6_bytes(self.ether_addr, dst_ether, &packet, tx_token);
                None
            }
            EtherIngress::Drop => None,
        }
    }

    fn parse_ip_or_process_neighbor<'pkt>(
        &self,
        data: &'pkt [u8],
        iface_cx: &mut Context,
    ) -> EtherIngress<'pkt> {
        // Parse the Ethernet header. Ignore the packet if the header is ill-formed.
        let Ok(frame) = EthernetFrame::new_checked(data) else {
            return EtherIngress::Drop;
        };
        let Ok(repr) = EthernetRepr::parse(&frame) else {
            return EtherIngress::Drop;
        };

        // Ignore the Ethernet frame if it is not sent to us.
        if !repr.dst_addr.is_broadcast()
            && !repr.dst_addr.is_multicast()
            && repr.dst_addr != self.ether_addr
        {
            return EtherIngress::Drop;
        }

        // Ignore the Ethernet frame if the protocol is not supported.
        match repr.ethertype {
            EthernetProtocol::Ipv4 => {
                let Ok(pkt) = Ipv4Packet::new_checked(frame.payload()) else {
                    return EtherIngress::Drop;
                };
                EtherIngress::Ip(IpPacket::Ipv4(pkt))
            }
            EthernetProtocol::Ipv6 => {
                let Ok(pkt) = Ipv6Packet::new_checked(frame.payload()) else {
                    return EtherIngress::Drop;
                };
                if let Some(ingress) = self.process_ndisc(&pkt, iface_cx, repr.src_addr) {
                    return ingress;
                }
                EtherIngress::Ip(IpPacket::Ipv6(pkt))
            }
            EthernetProtocol::Arp => {
                let Ok(pkt) = ArpPacket::new_checked(frame.payload()) else {
                    return EtherIngress::Drop;
                };
                let Ok(arp) = ArpRepr::parse(&pkt) else {
                    return EtherIngress::Drop;
                };
                match self.process_arp(&arp, iface_cx) {
                    Some(arp) => EtherIngress::Arp(arp),
                    None => EtherIngress::Drop,
                }
            }
            _ => EtherIngress::Drop,
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

    fn process_ndisc(
        &self,
        pkt: &Ipv6Packet<&[u8]>,
        iface_cx: &Context,
        src_ether: EthernetAddress,
    ) -> Option<EtherIngress<'static>> {
        let ipv6_repr = Ipv6Repr::parse(pkt).ok()?;
        if ipv6_repr.next_header != IpProtocol::Icmpv6 || ipv6_repr.hop_limit != 0xff {
            return None;
        }

        let icmp_pkt = Icmpv6Packet::new_checked(pkt.payload()).ok()?;
        let Icmpv6Repr::Ndisc(ndisc_repr) = Icmpv6Repr::parse(
            &ipv6_repr.src_addr,
            &ipv6_repr.dst_addr,
            &icmp_pkt,
            &iface_cx.checksum_caps(),
        )
        .ok()?
        else {
            return None;
        };

        match ndisc_repr {
            NdiscRepr::NeighborAdvert {
                flags,
                target_addr,
                lladdr: Some(lladdr),
            } if target_addr.x_is_unicast() => {
                let should_cache = flags.contains(NdiscNeighborFlags::OVERRIDE)
                    || !self.ndisc_table.lock().contains_key(&target_addr);
                let dst_ether = if should_cache {
                    self.cache_ipv6_neighbor(target_addr, lladdr);
                    Self::raw_hardware_addr_to_ether(lladdr)
                } else {
                    self.ndisc_table.lock().get(&target_addr).copied()
                };

                dst_ether.and_then(|dst_ether| {
                    self.take_pending_ipv6_packet(target_addr, iface_cx.now().total_millis())
                        .map(|packet| EtherIngress::PendingIpv6(dst_ether, packet))
                })
            }
            NdiscRepr::NeighborSolicit {
                target_addr,
                lladdr,
            } if target_addr.x_is_unicast() => {
                if let Some(lladdr) = lladdr {
                    self.cache_ipv6_neighbor(ipv6_repr.src_addr, lladdr);
                }

                if !self.is_solicited_node_addr(ipv6_repr.dst_addr, target_addr, iface_cx) {
                    return None;
                }

                // A solicitation from the unspecified address is sent during
                // duplicate address detection. The advertisement must then go to
                // the all-nodes multicast address with the solicited flag cleared
                // (RFC 4861, Section 7.2.4).
                let (dst_ip, dst_ether, solicited) = if ipv6_repr.src_addr.is_unspecified() {
                    let all_nodes =
                        Ipv6Address::from([0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1]);
                    (
                        all_nodes,
                        Self::ipv6_multicast_to_ethernet(all_nodes),
                        NdiscNeighborFlags::empty(),
                    )
                } else {
                    let dst_ether = self
                        .ndisc_table
                        .lock()
                        .get(&ipv6_repr.src_addr)
                        .copied()
                        .unwrap_or(src_ether);
                    (ipv6_repr.src_addr, dst_ether, NdiscNeighborFlags::SOLICITED)
                };

                Some(EtherIngress::Icmpv6(Icmpv6Message {
                    src_ip: target_addr,
                    dst_ip,
                    dst_ether,
                    repr: Icmpv6Repr::Ndisc(NdiscRepr::NeighborAdvert {
                        flags: solicited | NdiscNeighborFlags::OVERRIDE,
                        target_addr,
                        lladdr: Some(self.ether_addr.into()),
                    }),
                }))
            }
            _ => None,
        }
    }

    fn cache_ipv6_neighbor(&self, ip_addr: Ipv6Address, lladdr: RawHardwareAddress) {
        let Some(ether_addr) = Self::raw_hardware_addr_to_ether(lladdr) else {
            return;
        };

        if !ip_addr.x_is_unicast() || !ether_addr.is_unicast() {
            return;
        }

        self.ndisc_table.lock().insert(ip_addr, ether_addr);
    }

    fn take_pending_ipv6_packet(&self, ip_addr: Ipv6Address, now_ms: i64) -> Option<Vec<u8>> {
        let pending_packet = self.pending_ipv6_packets.lock().remove(&ip_addr)?;
        (pending_packet.expires_at_ms > now_ms).then_some(pending_packet.packet)
    }

    fn queue_pending_ipv6_packet(&self, ip_addr: Ipv6Address, packet: Vec<u8>, now_ms: i64) {
        let mut pending_packets = self.pending_ipv6_packets.lock();
        Self::remove_expired_pending_ipv6_packets(&mut pending_packets, now_ms);

        if pending_packets.contains_key(&ip_addr)
            || pending_packets.len() >= MAX_PENDING_IPV6_PACKETS
        {
            return;
        }

        pending_packets.insert(
            ip_addr,
            PendingIpv6Packet {
                packet,
                expires_at_ms: now_ms.saturating_add(PENDING_IPV6_PACKET_TIMEOUT_MS),
            },
        );
    }

    fn remove_expired_pending_ipv6_packets(
        pending_packets: &mut BTreeMap<Ipv6Address, PendingIpv6Packet>,
        now_ms: i64,
    ) {
        let expired_addrs = pending_packets
            .iter()
            .filter(|(_, pending_packet)| pending_packet.expires_at_ms <= now_ms)
            .map(|(ip_addr, _)| *ip_addr)
            .collect::<Vec<_>>();

        for ip_addr in expired_addrs {
            pending_packets.remove(&ip_addr);
        }
    }

    fn raw_hardware_addr_to_ether(lladdr: RawHardwareAddress) -> Option<EthernetAddress> {
        (lladdr.len() >= 6).then(|| EthernetAddress::from_bytes(&lladdr.as_bytes()[..6]))
    }

    fn is_solicited_node_addr(
        &self,
        dst_addr: Ipv6Address,
        target_addr: Ipv6Address,
        iface_cx: &Context,
    ) -> bool {
        iface_cx.ipv6_addr().is_some_and(|local_addr| {
            local_addr == target_addr
                && (dst_addr == target_addr || dst_addr == Self::solicited_node_addr(target_addr))
        })
    }

    fn solicited_node_addr(addr: Ipv6Address) -> Ipv6Address {
        let octets = addr.octets();
        Ipv6Address::from([
            0xff, 0x02, 0, 0, 0, 0, 0, 0, 0, 0, 0, 1, 0xff, octets[13], octets[14], octets[15],
        ])
    }

    fn ipv6_multicast_to_ethernet(addr: Ipv6Address) -> EthernetAddress {
        debug_assert!(addr.is_multicast());
        let octets = addr.octets();
        EthernetAddress::from_bytes(&[0x33, 0x33, octets[12], octets[13], octets[14], octets[15]])
    }

    fn dispatch<T: TxToken>(&self, pkt: &Packet, iface_cx: &mut Context, tx_token: T) {
        match self.resolve_ether_or_neighbor(pkt, iface_cx) {
            EtherTx::Ip(ether) => Self::emit_ip(&ether, pkt, &iface_cx.caps, tx_token),
            EtherTx::Arp(arp) => Self::emit_arp(&arp, tx_token),
            EtherTx::Icmpv6(message) => self.emit_icmpv6(&message, iface_cx, tx_token),
            EtherTx::Drop => (),
        }
    }

    fn resolve_ether_or_neighbor(&self, pkt: &Packet, iface_cx: &mut Context) -> EtherTx {
        // Resolve the next-hop IP address.
        let next_hop_ip = match iface_cx.route(&pkt.ip_repr().dst_addr(), iface_cx.now()) {
            Some(next_hop_ip) => next_hop_ip,
            None => return EtherTx::Drop,
        };

        match next_hop_ip {
            IpAddress::Ipv4(next_hop_ip) => {
                self.resolve_ipv4_ether_or_arp(pkt, iface_cx, next_hop_ip)
            }
            IpAddress::Ipv6(next_hop_ip) => {
                self.resolve_ipv6_ether_or_ndisc(pkt, iface_cx, next_hop_ip)
            }
        }
    }

    fn resolve_ipv4_ether_or_arp(
        &self,
        _pkt: &Packet,
        iface_cx: &Context,
        next_hop_ip: Ipv4Address,
    ) -> EtherTx {
        let next_hop_ether = if next_hop_ip.is_broadcast() {
            EthernetAddress::BROADCAST
        } else if let Some(next_hop_ether) = self.arp_table.lock().get(&next_hop_ip) {
            *next_hop_ether
        } else {
            return EtherTx::Arp(ArpRepr::EthernetIpv4 {
                operation: ArpOperation::Request,
                source_hardware_addr: self.ether_addr,
                source_protocol_addr: iface_cx.ipv4_addr().unwrap_or(Ipv4Address::UNSPECIFIED),
                target_hardware_addr: EthernetAddress::BROADCAST,
                target_protocol_addr: next_hop_ip,
            });
        };

        EtherTx::Ip(EthernetRepr {
            src_addr: self.ether_addr,
            dst_addr: next_hop_ether,
            ethertype: EthernetProtocol::Ipv4,
        })
    }

    fn resolve_ipv6_ether_or_ndisc(
        &self,
        pkt: &Packet,
        iface_cx: &Context,
        next_hop_ip: Ipv6Address,
    ) -> EtherTx {
        let IpAddress::Ipv6(dst_ip) = pkt.ip_repr().dst_addr() else {
            return EtherTx::Drop;
        };

        let next_hop_ether = if dst_ip.is_multicast() {
            Self::ipv6_multicast_to_ethernet(dst_ip)
        } else if let Some(next_hop_ether) = self.ndisc_table.lock().get(&next_hop_ip) {
            *next_hop_ether
        } else {
            let Some(src_ip) = iface_cx.ipv6_addr() else {
                return EtherTx::Drop;
            };
            let solicited_node_ip = Self::solicited_node_addr(next_hop_ip);
            self.queue_pending_ipv6_packet(
                next_hop_ip,
                Self::serialize_ip(pkt, &iface_cx.caps),
                iface_cx.now().total_millis(),
            );
            return EtherTx::Icmpv6(Icmpv6Message {
                src_ip,
                dst_ip: solicited_node_ip,
                dst_ether: Self::ipv6_multicast_to_ethernet(solicited_node_ip),
                repr: Icmpv6Repr::Ndisc(NdiscRepr::NeighborSolicit {
                    target_addr: next_hop_ip,
                    lladdr: Some(self.ether_addr.into()),
                }),
            });
        };

        EtherTx::Ip(EthernetRepr {
            src_addr: self.ether_addr,
            dst_addr: next_hop_ether,
            ethertype: EthernetProtocol::Ipv6,
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

    fn serialize_ip(ip_pkt: &Packet, caps: &DeviceCapabilities) -> Vec<u8> {
        let ip_repr = ip_pkt.ip_repr();
        let mut packet = vec![0; ip_repr.buffer_len()];
        ip_repr.emit(&mut packet, &caps.checksum);
        ip_pkt.emit_payload(&ip_repr, &mut packet[ip_repr.header_len()..], caps);
        packet
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

    /// Consumes the token and emits a serialized IPv6 packet.
    fn emit_ipv6_bytes<T: TxToken>(
        src_ether: EthernetAddress,
        dst_ether: EthernetAddress,
        ip_packet: &[u8],
        tx_token: T,
    ) {
        let ether_repr = EthernetRepr {
            src_addr: src_ether,
            dst_addr: dst_ether,
            ethertype: EthernetProtocol::Ipv6,
        };

        tx_token.consume(ether_repr.buffer_len() + ip_packet.len(), |buffer| {
            let mut frame = EthernetFrame::new_unchecked(buffer);
            ether_repr.emit(&mut frame);
            frame.payload_mut().copy_from_slice(ip_packet);
        });
    }

    /// Consumes the token and emits an ICMPv6 packet.
    fn emit_icmpv6<T: TxToken>(&self, message: &Icmpv6Message, iface_cx: &Context, tx_token: T) {
        let ip_repr = Ipv6Repr {
            src_addr: message.src_ip,
            dst_addr: message.dst_ip,
            next_header: IpProtocol::Icmpv6,
            payload_len: message.repr.buffer_len(),
            hop_limit: 0xff,
        };
        let ether_repr = EthernetRepr {
            src_addr: self.ether_addr,
            dst_addr: message.dst_ether,
            ethertype: EthernetProtocol::Ipv6,
        };

        tx_token.consume(
            ether_repr.buffer_len() + ip_repr.buffer_len() + message.repr.buffer_len(),
            |buffer| {
                let mut frame = EthernetFrame::new_unchecked(buffer);
                ether_repr.emit(&mut frame);

                let mut ip_packet = Ipv6Packet::new_unchecked(frame.payload_mut());
                ip_repr.emit(&mut ip_packet);

                let mut icmp_packet = Icmpv6Packet::new_unchecked(ip_packet.payload_mut());
                message.repr.emit(
                    &message.src_ip,
                    &message.dst_ip,
                    &mut icmp_packet,
                    &iface_cx.checksum_caps(),
                );
            },
        );
    }
}
