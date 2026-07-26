// SPDX-License-Identifier: MPL-2.0

use smoltcp::wire::{EthernetAddress, EthernetRepr};

use crate::{
    iface::wire::utils::Be16,
    packet::{LinkLayer, NetworkLayer, RxPacket, TxPacket},
};

pub fn parse(pkt: RxPacket<LinkLayer>) -> Option<(RxPacket<NetworkLayer>, EthernetRepr)> {
    let header = pkt.reader().read_val::<Header>().ok()?;

    let repr = EthernetRepr {
        src_addr: EthernetAddress(header.src),
        dst_addr: EthernetAddress(header.dst),
        ethertype: u16::from(header.typ).into(),
    };
    Some((pkt.peel(size_of::<Header>()), repr))
}

pub fn emit(mut pkt: TxPacket<NetworkLayer>, ethernet_repr: &EthernetRepr) -> TxPacket<LinkLayer> {
    let header = Header {
        dst: ethernet_repr.dst_addr.0,
        src: ethernet_repr.src_addr.0,
        typ: u16::from(ethernet_repr.ethertype).into(),
    };

    pkt.prepend(size_of::<Header>()).write_val(&header).unwrap();
    pkt.pack(size_of::<Header>())
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Header {
    dst: [u8; 6],
    src: [u8; 6],
    typ: Be16,
}
