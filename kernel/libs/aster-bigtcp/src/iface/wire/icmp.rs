// SPDX-License-Identifier: MPL-2.0

use ostd_pod::IntoBytes;
use smoltcp::wire::{Icmpv4DstUnreachable, Icmpv4Message};

use crate::{
    iface::wire::utils::Checksum,
    packet::{ApplicationLayer, TransportLayer, TxPacket},
};

pub const HEADER_LEN: usize = size_of::<Header>();

pub fn emit_dst_unreachable(
    mut packet: TxPacket<ApplicationLayer>,
    reason: Icmpv4DstUnreachable,
    csum: bool,
) -> TxPacket<TransportLayer> {
    let mut header = Header {
        type_: Icmpv4Message::DstUnreachable.into(),
        code: reason.into(),
        csum: 0,
        unused: 0,
    };

    if csum {
        header.csum = !Checksum::new()
            .with_bytes(header.as_bytes())
            .with_reader(packet.reader())
            .finish();
    }

    packet
        .prepend(size_of::<Header>())
        .write_val(&header)
        .unwrap();
    packet.pack(size_of::<Header>())
}

#[repr(C)]
#[derive(Clone, Copy, Debug, Pod)]
struct Header {
    type_: u8,
    code: u8,
    csum: u16,
    unused: u32,
}
