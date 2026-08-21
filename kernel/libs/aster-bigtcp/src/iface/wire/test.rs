// SPDX-License-Identifier: MPL-2.0

use alloc::{vec, vec::Vec};

use ostd::{
    mm::{VmReader, VmWriter},
    prelude::ktest,
};
use smoltcp::wire::{
    ArpOperation, ArpRepr, EthernetAddress, EthernetProtocol, EthernetRepr, IpProtocol, IpRepr,
    Ipv4Address, Ipv4Repr, Ipv6Address, Ipv6Repr, TcpControl, TcpRepr, TcpSeqNumber,
    TcpTimestampRepr, UdpRepr,
};

use super::{arp, ether, ip, tcp, udp};
use crate::packet::{
    AllocatedTxPacket, ApplicationLayer, Layer, NetworkLayer, RxPacket, TransportLayer, TxPacket,
};

fn tx_packet<L: Layer>(payload: &[u8]) -> TxPacket<L> {
    let allocated = AllocatedTxPacket::<L>::new(payload.len()).unwrap();
    let mut builder = allocated.to_builder();
    let written = builder.append().write(&mut VmReader::from(payload));
    assert_eq!(written, payload.len());
    builder.commit(written);
    builder.build()
}

fn packet_bytes<L>(packet: &RxPacket<L>) -> Vec<u8> {
    let mut bytes = vec![0; packet.len()];
    let read = packet
        .reader()
        .read(&mut VmWriter::from(bytes.as_mut_slice()));
    assert_eq!(read, bytes.len());
    bytes
}

#[ktest]
fn ethernet_round_trip() {
    let payload = [0x60, 0, 0, 0, 0, 0, 59, 64];
    let repr = EthernetRepr {
        src_addr: EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x01]),
        dst_addr: EthernetAddress([0x02, 0x00, 0x00, 0x00, 0x00, 0x02]),
        ethertype: EthernetProtocol::Ipv6,
    };

    let packet = ether::emit(tx_packet::<NetworkLayer>(&payload), &repr).to_rx_packet();
    let (packet, parsed) = ether::parse(packet).unwrap();
    assert_eq!(parsed, repr);
    assert_eq!(packet_bytes(&packet), payload);
}

#[ktest]
fn arp_round_trip() {
    let repr = ArpRepr::EthernetIpv4 {
        operation: ArpOperation::Request,
        source_hardware_addr: EthernetAddress([0x02, 0, 0, 0, 0, 1]),
        source_protocol_addr: Ipv4Address::new(192, 0, 2, 1),
        target_hardware_addr: EthernetAddress([0; 6]),
        target_protocol_addr: Ipv4Address::new(192, 0, 2, 2),
    };

    let packet = arp::emit(tx_packet::<TransportLayer>(&[]), &repr).to_rx_packet();
    let (packet, parsed) = arp::parse(packet).unwrap();
    assert_eq!(parsed, repr);
    assert_eq!(packet.len(), 0);
}

#[ktest]
fn udp_ipv4_round_trip() {
    let payload = [0x10, 0x20, 0x30, 0x40, 0x50];
    let udp_repr = UdpRepr {
        src_port: 49152,
        dst_port: 53,
    };
    let ip_repr = IpRepr::Ipv4(Ipv4Repr {
        src_addr: Ipv4Address::new(192, 0, 2, 1),
        dst_addr: Ipv4Address::new(198, 51, 100, 2),
        next_header: IpProtocol::Udp,
        payload_len: 8 + payload.len(),
        hop_limit: 64,
    });

    let packet = udp::emit(
        tx_packet::<ApplicationLayer>(&payload),
        &ip_repr,
        &udp_repr,
        true,
    );
    let packet = ip::emit(packet, &ip_repr, true).to_rx_packet();
    let (packet, parsed_ip) = ip::parse(packet, None, true).unwrap();
    let (packet, parsed_udp) = udp::parse(packet, &parsed_ip.inner, true).unwrap();
    assert_eq!(parsed_ip.inner, ip_repr);
    assert_eq!(parsed_udp, udp_repr);
    assert_eq!(packet_bytes(&packet), payload);
}

#[ktest]
fn udp_ipv6_round_trip() {
    let payload = [0xff, 0x00, 0x80, 0x7f, 0x01, 0x02, 0x03];
    let udp_repr = UdpRepr {
        src_port: 12345,
        dst_port: 443,
    };
    let ip_repr = IpRepr::Ipv6(Ipv6Repr {
        src_addr: Ipv6Address::new(0x2001, 0xdb8, 0, 1, 0, 0, 0, 1),
        dst_addr: Ipv6Address::new(0x2001, 0xdb8, 0, 2, 0, 0, 0, 2),
        next_header: IpProtocol::Udp,
        payload_len: 8 + payload.len(),
        hop_limit: 32,
    });

    let packet = udp::emit(
        tx_packet::<ApplicationLayer>(&payload),
        &ip_repr,
        &udp_repr,
        true,
    );
    let packet = ip::emit(packet, &ip_repr, true).to_rx_packet();
    let (packet, parsed_ip) = ip::parse(packet, None, true).unwrap();
    let (packet, parsed_udp) = udp::parse(packet, &parsed_ip.inner, true).unwrap();
    assert_eq!(parsed_ip.inner, ip_repr);
    assert_eq!(parsed_udp, udp_repr);
    assert_eq!(packet_bytes(&packet), payload);
}

#[ktest]
fn tcp_ipv4_round_trip() {
    let payload = [0x01, 0x02, 0x03, 0x04];
    let tcp_repr = TcpRepr {
        src_port: 12345,
        dst_port: 80,
        control: TcpControl::Syn,
        seq_number: TcpSeqNumber(0x1234_5678),
        ack_number: None,
        window_len: 8192,
        window_scale: Some(5),
        max_seg_size: Some(1460),
        sack_permitted: true,
        sack_ranges: [None; 3],
        timestamp: None,
        payload: &[],
    };
    let ip_repr = IpRepr::Ipv4(Ipv4Repr {
        src_addr: Ipv4Address::new(192, 0, 2, 10),
        dst_addr: Ipv4Address::new(198, 51, 100, 20),
        next_header: IpProtocol::Tcp,
        payload_len: tcp_repr.header_len() + payload.len(),
        hop_limit: 64,
    });

    let packet = tcp::emit(
        tx_packet::<ApplicationLayer>(&payload),
        &ip_repr,
        &tcp_repr,
        true,
    );
    let packet = ip::emit(packet, &ip_repr, true).to_rx_packet();
    let (packet, parsed_ip) = ip::parse(packet, None, true).unwrap();
    let (packet, parsed_tcp) = tcp::parse(packet, &parsed_ip.inner, true).unwrap();
    assert_eq!(parsed_ip.inner, ip_repr);
    assert_eq!(parsed_tcp, tcp_repr);
    assert_eq!(packet_bytes(&packet), payload);
}

#[ktest]
fn tcp_ipv6_round_trip() {
    let payload = [0xde, 0xad, 0xbe, 0xef, 0x01];
    let tcp_repr = TcpRepr {
        src_port: 45678,
        dst_port: 443,
        control: TcpControl::Psh,
        seq_number: TcpSeqNumber(-1234567),
        ack_number: Some(TcpSeqNumber(7654321)),
        window_len: 32768,
        window_scale: Some(7),
        max_seg_size: Some(1460),
        sack_permitted: false,
        sack_ranges: [Some((1000, 2000)), Some((3000, 4500)), None],
        timestamp: Some(TcpTimestampRepr::new(0x1234_5678, 0x9abc_def0)),
        payload: &[],
    };
    let ip_repr = IpRepr::Ipv6(Ipv6Repr {
        src_addr: Ipv6Address::new(0x2001, 0xdb8, 1, 0, 0, 0, 0, 1),
        dst_addr: Ipv6Address::new(0x2001, 0xdb8, 2, 0, 0, 0, 0, 2),
        next_header: IpProtocol::Tcp,
        payload_len: tcp_repr.header_len() + payload.len(),
        hop_limit: 255,
    });

    let packet = tcp::emit(
        tx_packet::<ApplicationLayer>(&payload),
        &ip_repr,
        &tcp_repr,
        true,
    );
    let packet = ip::emit(packet, &ip_repr, true).to_rx_packet();
    let (packet, parsed_ip) = ip::parse(packet, None, true).unwrap();
    let (packet, parsed_tcp) = tcp::parse(packet, &parsed_ip.inner, true).unwrap();
    assert_eq!(parsed_ip.inner, ip_repr);
    assert_eq!(parsed_tcp, tcp_repr);
    assert_eq!(packet_bytes(&packet), payload);
}
