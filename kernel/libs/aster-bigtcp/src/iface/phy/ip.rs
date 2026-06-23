// SPDX-License-Identifier: MPL-2.0

use alloc::{ffi::CString, sync::Arc};

use smoltcp::{
    iface::Config,
    phy::{Device, TxToken},
    wire::{self, Ipv4Cidr, Ipv4Packet, Ipv6Cidr, Ipv6Packet},
};

use crate::{
    device::WithDevice,
    ext::Ext,
    iface::{
        Iface, ScheduleNextPoll,
        common::{IfaceCommon, InterfaceFlags, InterfaceType, IpPacket},
        iface::internal::IfaceInternal,
        time::get_network_timestamp,
    },
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
        name: CString,
        sched_poll: E::ScheduleNextPoll,
        type_: InterfaceType,
        flags: InterfaceFlags,
    ) -> Arc<Self> {
        let interface = driver.with(|device| {
            let config = Config::new(wire::HardwareAddress::Ip);
            let now = get_network_timestamp();

            let mut interface = smoltcp::iface::Interface::new(config, device, now);
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
    fn poll(&self) {
        self.driver.with(|device| {
            let mut next_poll = self.common.poll(
                device,
                |data, _iface_cx, tx_token| {
                    if data.is_empty() {
                        return None;
                    }
                    let version = data[0] >> 4;

                    if version == 4 {
                        let pkt = Ipv4Packet::new_checked(data).ok()?;
                        Some((IpPacket::Ipv4(pkt), tx_token))
                    } else if version == 6 {
                        let pkt = Ipv6Packet::new_checked(data).ok()?;
                        Some((IpPacket::Ipv6(pkt), tx_token))
                    } else {
                        None
                    }
                },
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

            // For loopback devices, we need to poll again to process packets just sent
            if self.common.type_() == InterfaceType::LOOPBACK {
                let second_next_poll = self.common.poll(
                    device,
                    |data, _iface_cx, tx_token| {
                        if data.is_empty() {
                            return None;
                        }
                        let version = data[0] >> 4;

                        if version == 4 {
                            let pkt = Ipv4Packet::new_checked(data).ok()?;
                            Some((IpPacket::Ipv4(pkt), tx_token))
                        } else if version == 6 {
                            let pkt = Ipv6Packet::new_checked(data).ok()?;
                            Some((IpPacket::Ipv6(pkt), tx_token))
                        } else {
                            None
                        }
                    },
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
                // Take the earlier of the two next poll times
                if let Some(first) = next_poll {
                    if let Some(second) = second_next_poll
                        && second < first
                    {
                        next_poll = Some(second);
                    }
                } else {
                    next_poll = second_next_poll;
                }
            }

            // For loopback devices, ensure there's always a poll scheduled (at least every 100ms)
            if self.common.type_() == InterfaceType::LOOPBACK && next_poll.is_none() {
                let now_ms = get_network_timestamp().total_millis() as u64;
                next_poll = Some(now_ms + 100); // 100ms
            }

            self.common.sched_poll().schedule_next_poll(next_poll);
        });
    }

    fn mtu(&self) -> usize {
        self.driver
            .with(|device| device.capabilities().max_transmission_unit)
    }
}
