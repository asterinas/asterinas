// SPDX-License-Identifier: MPL-2.0

use smoltcp::wire::{ArpHardware, ArpRepr, EthernetAddress, EthernetProtocol, Ipv4Address};

use crate::{
    iface::wire::utils::Be16,
    packet::{NetworkLayer, RxPacket, TransportLayer, TxPacket},
};

pub fn parse(mut pkt: RxPacket<NetworkLayer>) -> Option<(RxPacket<TransportLayer>, ArpRepr)> {
    let header = pkt.reader().read_val::<Header>().ok()?;

    if ArpHardware::from(u16::from(header.hardware_type)) != ArpHardware::Ethernet
        || EthernetProtocol::from(u16::from(header.protocol_type)) != EthernetProtocol::Ipv4
        || header.hardware_len != 6
        || header.protocol_len != 4
    {
        return None;
    }

    pkt.truncate(size_of::<Header>());

    let repr = ArpRepr::EthernetIpv4 {
        operation: u16::from(header.operation).into(),
        source_hardware_addr: EthernetAddress(header.source_hardware_addr),
        source_protocol_addr: Ipv4Address::from(header.source_protocol_addr),
        target_hardware_addr: EthernetAddress(header.target_hardware_addr),
        target_protocol_addr: Ipv4Address::from(header.target_protocol_addr),
    };
    Some((pkt.peel(size_of::<Header>()), repr))
}

pub fn emit(mut pkt: TxPacket<TransportLayer>, arp_repr: &ArpRepr) -> TxPacket<NetworkLayer> {
    debug_assert_eq!(pkt.len(), 0);

    let header = match *arp_repr {
        ArpRepr::EthernetIpv4 {
            operation,
            source_hardware_addr,
            source_protocol_addr,
            target_hardware_addr,
            target_protocol_addr,
        } => Header {
            hardware_type: u16::from(ArpHardware::Ethernet).into(),
            protocol_type: u16::from(EthernetProtocol::Ipv4).into(),
            hardware_len: 6,
            protocol_len: 4,
            operation: u16::from(operation).into(),
            source_hardware_addr: source_hardware_addr.0,
            source_protocol_addr: source_protocol_addr.octets(),
            target_hardware_addr: target_hardware_addr.0,
            target_protocol_addr: target_protocol_addr.octets(),
        },
        _ => unreachable!(),
    };

    pkt.prepend(size_of::<Header>()).write_val(&header).unwrap();
    pkt.pack(size_of::<Header>())
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Header {
    hardware_type: Be16,
    protocol_type: Be16,
    hardware_len: u8,
    protocol_len: u8,
    operation: Be16,
    source_hardware_addr: [u8; 6],
    source_protocol_addr: [u8; 4],
    target_hardware_addr: [u8; 6],
    target_protocol_addr: [u8; 4],
}
