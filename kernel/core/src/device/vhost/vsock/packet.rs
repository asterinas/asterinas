// SPDX-License-Identifier: MPL-2.0

//! Bounded host packets and their progress through guest receive buffers.

use aster_virtio::device::socket::header::VirtioVsockHdr;

use crate::prelude::*;

pub(super) const HEADER_LEN: usize = size_of::<VirtioVsockHdr>();
pub(super) const MAX_PAYLOAD_LEN: usize = 64 * 1024;
const MAX_DATA_BYTES: usize = 1024 * 1024;
const MAX_DATA_PACKETS: usize = 256;
const CONTROL_RESERVE: usize = 64;

pub(super) struct Packet {
    pub header: VirtioVsockHdr,
    pub payload: Box<[u8]>,
}

impl Packet {
    pub fn new(header: VirtioVsockHdr, payload: &[u8]) -> Result<Arc<Self>> {
        if payload.len() > MAX_PAYLOAD_LEN || header.len as usize != payload.len() {
            return_errno_with_message!(Errno::EINVAL, "invalid vhost-vsock payload length");
        }
        Ok(Arc::new(Self {
            header,
            payload: payload.into(),
        }))
    }

    pub fn header_for_fragment(&self, offset: usize, capacity: usize) -> Result<VirtioVsockHdr> {
        if capacity < HEADER_LEN || (capacity == HEADER_LEN && offset < self.payload.len()) {
            return_errno_with_message!(Errno::EINVAL, "the vsock receive buffer is too short");
        }
        let mut header = self.header;
        header.len = (self.payload.len() - offset).min(capacity - HEADER_LEN) as u32;
        Ok(header)
    }
}

pub(super) struct PendingPackets {
    pub is_active: bool,
    pub is_running: bool,
    pub generation: u64,
    pub failed: bool,
    packets: VecDeque<(Arc<Packet>, usize)>,
    bytes: usize,
    reservations: usize,
}

impl PendingPackets {
    pub fn new() -> Self {
        Self {
            is_active: true,
            is_running: false,
            generation: 0,
            failed: false,
            packets: VecDeque::new(),
            bytes: 0,
            reservations: 0,
        }
    }

    pub fn has_data_room(&self) -> bool {
        self.is_active
            && self.packets.len() + self.reservations < MAX_DATA_PACKETS
            && self.bytes + HEADER_LEN + MAX_PAYLOAD_LEN <= MAX_DATA_BYTES
    }

    pub fn has_control_room(&self) -> bool {
        self.is_active
            && self.packets.len() + self.reservations + 4 < MAX_DATA_PACKETS + CONTROL_RESERVE
            && self.bytes + 4 * HEADER_LEN <= MAX_DATA_BYTES + CONTROL_RESERVE * HEADER_LEN
    }

    pub fn reserve(&mut self, len: usize) -> bool {
        if !self.has_data_room() {
            return false;
        }
        self.reservations += 1;
        self.bytes += HEADER_LEN + len;
        true
    }

    pub fn release(&mut self, len: usize) {
        self.reservations -= 1;
        self.bytes -= HEADER_LEN + len;
    }

    pub fn push_reserved(&mut self, packet: Arc<Packet>) {
        self.reservations -= 1;
        self.packets.push_back((packet, 0));
    }

    pub fn push(&mut self, packet: Arc<Packet>) -> bool {
        let len = HEADER_LEN + packet.payload.len();
        let (max_packets, max_bytes) = if packet.payload.is_empty() {
            (
                MAX_DATA_PACKETS + CONTROL_RESERVE,
                MAX_DATA_BYTES + CONTROL_RESERVE * HEADER_LEN,
            )
        } else {
            (MAX_DATA_PACKETS, MAX_DATA_BYTES)
        };
        if !self.is_active
            || self.packets.len() + self.reservations >= max_packets
            || self.bytes + len > max_bytes
        {
            return false;
        }
        self.bytes += len;
        self.packets.push_back((packet, 0));
        true
    }

    pub fn front(&self) -> Option<(Arc<Packet>, usize)> {
        self.packets
            .front()
            .map(|(packet, offset)| (packet.clone(), *offset))
    }

    pub fn complete_fragment(&mut self, len: usize) {
        let (packet, offset) = self.packets.front_mut().unwrap();
        *offset += len;
        if *offset == packet.payload.len() {
            self.bytes -= HEADER_LEN + packet.payload.len();
            self.packets.pop_front();
        }
    }

    pub fn discard(&mut self) {
        // In-flight send reservations can outlive a worker failure. Their Drop
        // handlers retain responsibility for returning the reserved capacity.
        for (packet, _) in self.packets.drain(..) {
            self.bytes -= HEADER_LEN + packet.payload.len();
        }
    }
}

pub(super) fn decode_header(bytes: &[u8; HEADER_LEN]) -> VirtioVsockHdr {
    let header = VirtioVsockHdr::from_bytes(bytes);
    VirtioVsockHdr {
        src_cid: u64::from_le(header.src_cid),
        dst_cid: u64::from_le(header.dst_cid),
        src_port: u32::from_le(header.src_port),
        dst_port: u32::from_le(header.dst_port),
        len: u32::from_le(header.len),
        type_: u16::from_le(header.type_),
        op: u16::from_le(header.op),
        flags: u32::from_le(header.flags),
        buf_alloc: u32::from_le(header.buf_alloc),
        fwd_cnt: u32::from_le(header.fwd_cnt),
    }
}

pub(super) fn encode_header(header: VirtioVsockHdr) -> VirtioVsockHdr {
    VirtioVsockHdr {
        src_cid: header.src_cid.to_le(),
        dst_cid: header.dst_cid.to_le(),
        src_port: header.src_port.to_le(),
        dst_port: header.dst_port.to_le(),
        len: header.len.to_le(),
        type_: header.type_.to_le(),
        op: header.op.to_le(),
        flags: header.flags.to_le(),
        buf_alloc: header.buf_alloc.to_le(),
        fwd_cnt: header.fwd_cnt.to_le(),
    }
}
