// SPDX-License-Identifier: MPL-2.0

use int_to_c_enum::TryFromInt;
use ostd::mm::{Infallible, VmReader, VmWriter};
use ostd_pod::IntoBytes;
use smoltcp::wire::{IpProtocol, IpRepr, TcpControl, TcpRepr, TcpSeqNumber, TcpTimestampRepr};

use crate::{
    iface::wire::utils::{Be16, Be32, Checksum},
    packet::{ApplicationLayer, RxPacket, TransportLayer, TxPacket},
};

pub fn parse(
    pkt: RxPacket<TransportLayer>,
    ip_repr: &IpRepr,
    csum: bool,
) -> Option<(RxPacket<ApplicationLayer>, TcpRepr<'static>)> {
    let header = pkt.reader().read_val::<Header>().ok()?;

    let header_len = header.header_len();
    if header_len < size_of::<Header>() || header_len > pkt.len() {
        return None;
    }

    let src_port = u16::from(header.src);
    let dst_port = u16::from(header.dst);
    if src_port == 0 || dst_port == 0 {
        // In practice, zero port numbers are not valid because none of the operating systems in
        // production can bind to them. They are reserved by the IANA:
        // <https://www.iana.org/assignments/service-names-port-numbers/service-names-port-numbers.xhtml>.
        return None;
    }

    let flags = Flags::from_bits_truncate(header.flags);
    let control = match (
        flags.contains(Flags::SYN),
        flags.contains(Flags::FIN),
        flags.contains(Flags::RST),
        flags.contains(Flags::PSH),
    ) {
        (false, false, false, false) => TcpControl::None,
        (false, false, false, true) => TcpControl::Psh,
        (true, false, false, _) => TcpControl::Syn,
        (false, true, false, _) => TcpControl::Fin,
        (false, false, true, _) => TcpControl::Rst,
        _ => return None,
    };

    let ack_number = flags
        .contains(Flags::ACK)
        .then(|| TcpSeqNumber(u32::from(header.ack) as i32));

    if csum
        && Checksum::new()
            .with_pseudo(ip_repr, pkt.len())
            .with_reader(pkt.reader())
            .finish()
            != u16::MAX
    {
        return None;
    }

    let mut repr = TcpRepr {
        src_port,
        dst_port,
        control,
        seq_number: TcpSeqNumber(u32::from(header.seq) as i32),
        ack_number,
        window_len: header.win.into(),
        window_scale: None,
        max_seg_size: None,
        sack_permitted: false,
        sack_ranges: [None; 3],
        timestamp: None,
        payload: &[],
    };

    if header_len != size_of::<Header>() {
        let mut options = pkt.reader();
        options
            .skip(size_of::<Header>())
            .limit(header_len - size_of::<Header>());
        if !parse_options(options, &mut repr) {
            return None;
        }
    }

    Some((pkt.peel(header_len), repr))
}

#[must_use]
fn parse_options(mut options: VmReader<Infallible>, repr: &mut TcpRepr<'static>) -> bool {
    while let Ok(kind) = options.read_val::<u8>() {
        let kind = OptionKind::try_from(kind).ok();
        match kind {
            Some(OptionKind::End) => break,
            Some(OptionKind::Nop) => continue,
            _ => {}
        }

        let Ok(option_len) = options.read_val::<u8>() else {
            return false;
        };
        let option_len = option_len as usize;
        if option_len < 2 || option_len - 2 > options.remain() {
            return false;
        }

        match (kind, option_len) {
            (Some(OptionKind::MaxSegmentSize), 4) => {
                repr.max_seg_size = Some(options.read_val::<Be16>().unwrap().into());
            }
            (Some(OptionKind::MaxSegmentSize), _) => return false,

            (Some(OptionKind::WindowScale), 3) => {
                // "Thus, the shift count MUST be limited to 14 (which allows windows of 2^30 = 1
                // GiB).  If a Window Scale option is received with a shift.cnt value larger than
                // 14, the TCP SHOULD log the error but MUST use 14 instead of the specified value."
                // Reference: <https://datatracker.ietf.org/doc/html/rfc7323#section-2.3>.
                repr.window_scale = Some(options.read_val::<u8>().unwrap().min(14));
            }
            (Some(OptionKind::WindowScale), _) => return false,

            (Some(OptionKind::SackPermitted), 2) => repr.sack_permitted = true,
            (Some(OptionKind::SackPermitted), _) => return false,

            (Some(OptionKind::SackRange), len) if len >= 10 && (len - 2) % 8 == 0 => {
                repr.sack_ranges = [None; 3];
                for index in 0..(len - 2) / 8 {
                    let left = options.read_val::<Be32>().unwrap().into();
                    let right = options.read_val::<Be32>().unwrap().into();
                    if let Some(range) = repr.sack_ranges.get_mut(index) {
                        *range = Some((left, right));
                    }
                }
            }
            (Some(OptionKind::SackRange), _) => return false,

            (Some(OptionKind::Timestamp), 10) => {
                let tsval = options.read_val::<Be32>().unwrap().into();
                let tsecr = options.read_val::<Be32>().unwrap().into();
                repr.timestamp = Some(TcpTimestampRepr::new(tsval, tsecr));
            }
            (Some(OptionKind::Timestamp), _) => return false,

            (Some(OptionKind::End | OptionKind::Nop), _) => unreachable!(),
            (None, len) => {
                options.skip(len - 2);
            }
        }
    }

    true
}

pub fn emit(
    mut pkt: TxPacket<ApplicationLayer>,
    ip_repr: &IpRepr,
    tcp_repr: &TcpRepr<'_>,
    csum: bool,
) -> TxPacket<TransportLayer> {
    debug_assert_eq!(ip_repr.next_header(), IpProtocol::Tcp);

    let header_len = tcp_repr.header_len();
    let packet_len = header_len + pkt.len();
    debug_assert_eq!(ip_repr.payload_len(), packet_len);

    let mut flags = match tcp_repr.control {
        TcpControl::None => Flags::empty(),
        TcpControl::Psh => Flags::PSH,
        TcpControl::Syn => Flags::SYN,
        TcpControl::Fin => Flags::FIN,
        TcpControl::Rst => Flags::RST,
    };
    if tcp_repr.ack_number.is_some() {
        flags.insert(Flags::ACK);
    }

    let mut header = Header {
        src: tcp_repr.src_port.into(),
        dst: tcp_repr.dst_port.into(),
        seq: (tcp_repr.seq_number.0 as u32).into(),
        ack: (tcp_repr.ack_number.unwrap_or(TcpSeqNumber(0)).0 as u32).into(),
        off: ((header_len / 4) as u8) << 4,
        flags: flags.bits(),
        win: tcp_repr.window_len.into(),
        csum: 0,
        urg: 0u16.into(),
    };

    let option_len = header_len - size_of::<Header>();
    if option_len != 0 {
        let writer = pkt.prepend(option_len);
        emit_options(writer, tcp_repr);
    }

    if csum {
        let csum_val = !Checksum::new()
            .with_pseudo(ip_repr, packet_len)
            .with_bytes(header.as_bytes())
            .with_reader(pkt.reader_with_header(option_len))
            .finish();
        header.csum = csum_val;
    }

    pkt.prepend(header_len).write_val(&header).unwrap();
    pkt.pack(header_len)
}

fn emit_options(mut writer: VmWriter<Infallible>, repr: &TcpRepr<'_>) {
    if let Some(value) = repr.max_seg_size {
        writer
            .write_val(&[OptionKind::MaxSegmentSize as u8, 4])
            .unwrap();
        writer.write_val(&Be16::from(value)).unwrap();
    }

    if let Some(value) = repr.window_scale {
        writer
            .write_val(&[OptionKind::WindowScale as u8, 3, value])
            .unwrap();
    }

    if repr.sack_permitted {
        writer
            .write_val(&[OptionKind::SackPermitted as u8, 2])
            .unwrap();
    }

    if repr.sack_ranges.iter().any(Option::is_some) {
        let range_count = repr.sack_ranges.iter().flatten().count();
        writer
            .write_val(&[OptionKind::SackRange as u8, (range_count * 8 + 2) as u8])
            .unwrap();
        for &(left, right) in repr.sack_ranges.iter().flatten() {
            writer.write_val(&Be32::from(left)).unwrap();
            writer.write_val(&Be32::from(right)).unwrap();
        }
    }

    if let Some(timestamp) = repr.timestamp {
        writer
            .write_val(&[OptionKind::Timestamp as u8, 10])
            .unwrap();
        writer.write_val(&Be32::from(timestamp.tsval)).unwrap();
        writer.write_val(&Be32::from(timestamp.tsecr)).unwrap();
    }

    let padding_len = writer.avail();
    writer.fill_zeros(padding_len);
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Header {
    src: Be16,
    dst: Be16,
    seq: Be32,
    ack: Be32,
    off: u8,
    flags: u8,
    win: Be16,
    csum: u16,
    urg: Be16,
}

impl Header {
    fn header_len(self) -> usize {
        (self.off as usize >> 4) * 4
    }
}

bitflags::bitflags! {
    struct Flags: u8 {
        const FIN = 1 << 0;
        const SYN = 1 << 1;
        const RST = 1 << 2;
        const PSH = 1 << 3;
        const ACK = 1 << 4;
        const URG = 1 << 5;
        const ECE = 1 << 6;
        const CWR = 1 << 7;
    }
}

#[repr(u8)]
#[derive(Clone, Copy, Debug, TryFromInt)]
enum OptionKind {
    End = 0,
    Nop = 1,
    MaxSegmentSize = 2,
    WindowScale = 3,
    SackPermitted = 4,
    SackRange = 5,
    Timestamp = 8,
}
