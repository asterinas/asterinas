// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;
use ostd_pod::IntoBytes;
use smoltcp::wire::{IpRepr, Ipv4Repr, Ipv6Repr};

use crate::{
    iface::wire::utils::{Be16, Checksum},
    packet::{NetworkLayer, RxPacket, TransportLayer, TxPacket},
};

pub fn parse(
    pkt: RxPacket<NetworkLayer>,
    ver: Option<Version>,
    csum: bool,
) -> Option<(RxPacket<TransportLayer>, IpReprWithLen)> {
    let pkt_ver = {
        let byte0 = pkt.reader().read_val::<VersionAndIhl>().ok()?;
        byte0.version()?
    };

    if ver.is_some_and(|v| v != pkt_ver) {
        return None;
    }

    match pkt_ver {
        Version::V4 => do_parse_v4(pkt, csum),
        Version::V6 => do_parse_v6(pkt),
    }
}

fn do_parse_v4(
    mut pkt: RxPacket<NetworkLayer>,
    csum: bool,
) -> Option<(RxPacket<TransportLayer>, IpReprWithLen)> {
    let header = pkt.reader().read_val::<HeaderV4>().ok()?;
    debug_assert_eq!(header.ver_ihl.version(), Some(Version::V4));

    let header_len = header.ver_ihl.header_len();
    let total_len = u16::from(header.totlen) as usize;
    if header_len < size_of::<HeaderV4>() || total_len < header_len || total_len > pkt.len() {
        return None;
    }
    pkt.truncate(total_len);

    if header.flags_frag.flags().contains(Flags::MF) || header.flags_frag.fragment_offset() != 0 {
        // IP fragments are not currently supported.
        return None;
    }

    if csum {
        let mut reader = pkt.reader();
        reader.limit(header_len);
        if Checksum::new().with_reader(reader).finish() != u16::MAX {
            return None;
        }
    }

    let inner = Ipv4Repr {
        src_addr: header.src.into(),
        dst_addr: header.dst.into(),
        next_header: header.proto.into(),
        payload_len: total_len - header_len,
        hop_limit: header.ttl,
    };
    let repr = IpReprWithLen {
        inner: IpRepr::Ipv4(inner),
        header_len,
    };
    Some((pkt.peel(header_len), repr))
}

fn do_parse_v6(
    mut pkt: RxPacket<NetworkLayer>,
) -> Option<(RxPacket<TransportLayer>, IpReprWithLen)> {
    let header = pkt.reader().read_val::<HeaderV6>().ok()?;
    debug_assert_eq!(header.ver_flow.version(), Some(Version::V6));

    let payload_len = u16::from(header.len) as usize;
    let total_len = size_of::<HeaderV6>() + payload_len;
    if total_len > pkt.len() {
        return None;
    }
    pkt.truncate(total_len);

    let inner = Ipv6Repr {
        src_addr: header.src.into(),
        dst_addr: header.dst.into(),
        next_header: header.next.into(),
        payload_len,
        hop_limit: header.hops,
    };
    let repr = IpReprWithLen {
        inner: IpRepr::Ipv6(inner),
        header_len: size_of::<HeaderV6>(),
    };
    Some((pkt.peel(size_of::<HeaderV6>()), repr))
}

pub fn emit(pkt: TxPacket<TransportLayer>, ip_repr: &IpRepr, csum: bool) -> TxPacket<NetworkLayer> {
    match ip_repr {
        IpRepr::Ipv4(ipv4_repr) => emit_v4(pkt, ipv4_repr, csum),
        IpRepr::Ipv6(ipv6_repr) => emit_v6(pkt, ipv6_repr),
    }
}

fn emit_v4(
    mut pkt: TxPacket<TransportLayer>,
    ipv4_repr: &Ipv4Repr,
    csum: bool,
) -> TxPacket<NetworkLayer> {
    debug_assert_eq!(ipv4_repr.payload_len, pkt.len());

    let mut header = HeaderV4 {
        ver_ihl: VersionAndIhl::new(Version::V4, size_of::<HeaderV4>()),
        dscp_ecn: DscpAndEcn::new(Dscp::Cs0, Ecn::NotEct),
        totlen: ((ipv4_repr.payload_len + size_of::<HeaderV4>()) as u16).into(),
        // If necessary, we should fill the identification field.
        ident: 0u16.into(),
        flags_frag: FlagsAndFragmentOffset::new(Flags::DF, 0),
        ttl: ipv4_repr.hop_limit,
        proto: ipv4_repr.next_header.into(),
        csum: 0,
        src: ipv4_repr.src_addr.octets(),
        dst: ipv4_repr.dst_addr.octets(),
    };

    if csum {
        header.csum = !Checksum::new().with_bytes(header.as_bytes()).finish();
    }

    pkt.prepend(size_of::<HeaderV4>())
        .write_val(&header)
        .unwrap();
    pkt.pack(size_of::<HeaderV4>())
}

fn emit_v6(mut pkt: TxPacket<TransportLayer>, ipv6_repr: &Ipv6Repr) -> TxPacket<NetworkLayer> {
    debug_assert_eq!(ipv6_repr.payload_len, pkt.len());

    let header = HeaderV6 {
        // If necessary, we should fill the flow label field.
        ver_flow: VersionToFlowLabel::new(Version::V6, Dscp::Cs0, Ecn::NotEct, 0),
        len: (ipv6_repr.payload_len as u16).into(),
        next: ipv6_repr.next_header.into(),
        hops: ipv6_repr.hop_limit,
        src: ipv6_repr.src_addr.octets(),
        dst: ipv6_repr.dst_addr.octets(),
    };

    pkt.prepend(size_of::<HeaderV6>())
        .write_val(&header)
        .unwrap();
    pkt.pack(size_of::<HeaderV6>())
}

#[derive(Clone, Debug)]
pub struct IpReprWithLen {
    pub inner: IpRepr,
    pub header_len: usize,
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct HeaderV4 {
    ver_ihl: VersionAndIhl,
    dscp_ecn: DscpAndEcn,
    totlen: Be16,
    ident: Be16,
    flags_frag: FlagsAndFragmentOffset,
    ttl: u8,
    proto: u8,
    csum: u16,
    src: [u8; 4],
    dst: [u8; 4],
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
struct VersionAndIhl(u8);

#[repr(u8)]
#[derive(Clone, Copy, Debug, Eq, PartialEq, TryFromInt)]
pub enum Version {
    V4 = 4,
    V6 = 6,
}

impl VersionAndIhl {
    fn new(version: Version, header_len: usize) -> Self {
        Self(((version as u8) << 4) | (header_len as u8 / 4))
    }

    fn version(self) -> Option<Version> {
        Version::try_from(self.0 >> 4).ok()
    }

    fn header_len(self) -> usize {
        (self.0 as usize & 0xf) * 4
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
struct DscpAndEcn(u8);

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
enum Dscp {
    Cs0 = 0,
    // Add other variants (e.g., `CS1`) when needed.
}

#[repr(u8)]
#[derive(Clone, Copy, Debug)]
enum Ecn {
    NotEct = 0b00,
    // Add other variants (e.g., `Ect1`) when needed.
}

impl DscpAndEcn {
    fn new(dscp: Dscp, ecn: Ecn) -> Self {
        Self(((dscp as u8) << 2) | ecn as u8)
    }
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
struct FlagsAndFragmentOffset([u8; 2]);

bitflags::bitflags! {
    struct Flags: u8 {
        const R  = 1 << 2;
        const DF = 1 << 1;
        const MF = 1 << 0;
    }
}

impl FlagsAndFragmentOffset {
    fn new(flags: Flags, fragment_offset: usize) -> Self {
        debug_assert_eq!(fragment_offset >> 13, 0);
        Self([
            (flags.bits() << 5) | ((fragment_offset >> 8) as u8),
            fragment_offset as u8,
        ])
    }

    fn flags(self) -> Flags {
        Flags::from_bits_truncate(self.0[0] >> 5)
    }

    fn fragment_offset(self) -> usize {
        self.0[1] as usize | ((self.0[0] as usize & 0x1f) << 8)
    }
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct HeaderV6 {
    ver_flow: VersionToFlowLabel,
    len: Be16,
    next: u8,
    hops: u8,
    src: [u8; 16],
    dst: [u8; 16],
}

#[repr(transparent)]
#[derive(Clone, Copy, Debug, Pod)]
struct VersionToFlowLabel([u8; 4]);

impl VersionToFlowLabel {
    fn new(version: Version, dscp: Dscp, ecn: Ecn, flow_label: u32) -> Self {
        debug_assert_eq!(flow_label >> 20, 0);
        Self([
            ((version as u8) << 4) | (dscp as u8 >> 2),
            ((dscp as u8) << 6) | ((ecn as u8) << 4) | ((flow_label >> 16) as u8),
            (flow_label >> 8) as u8,
            flow_label as u8,
        ])
    }

    fn version(self) -> Option<Version> {
        Version::try_from(self.0[0] >> 4).ok()
    }
}
