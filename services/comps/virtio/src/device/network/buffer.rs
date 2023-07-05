use align_ext::AlignExt;
use jinux_frame::{
    config::PAGE_SIZE,
    vm::{OwnedFrames, VmIo},
};
use pod::Pod;

use crate::device::network::header::VIRTIO_NET_HDR_LEN;

use super::header::VirtioNetHdr;

/// Buffer for receive packet
pub struct RxBuffer {
    /// Packet Buffer, length align 8.
    buf: OwnedFrames,
    /// Packet len
    packet_len: usize,
}

impl RxBuffer {
    pub fn new(len: usize) -> Self {
        let num_pages = len.align_up(8) / PAGE_SIZE;
        let buf = OwnedFrames::new(num_pages).unwrap();
        Self { buf, packet_len: 0 }
    }

    pub const fn packet_len(&self) -> usize {
        self.packet_len
    }

    pub fn set_packet_len(&mut self, packet_len: usize) {
        self.packet_len = packet_len;
    }

    pub fn buf(&self) -> &[u8] {
        self.buf.as_slice()
    }

    pub fn buf_mut(&mut self) -> &mut [u8] {
        self.buf.as_slice_mut()
    }

    /// Packet payload slice, which is inner buffer excluding VirtioNetHdr.
    pub fn packet(&self) -> &[u8] {
        debug_assert!(VIRTIO_NET_HDR_LEN + self.packet_len <= self.buf.as_slice().len());
        &self.buf.as_slice()[VIRTIO_NET_HDR_LEN..VIRTIO_NET_HDR_LEN + self.packet_len]
    }

    /// Mutable packet payload slice.
    pub fn packet_mut(&mut self) -> &mut [u8] {
        debug_assert!(VIRTIO_NET_HDR_LEN + self.packet_len <= self.buf.as_slice().len());
        &mut self.buf.as_slice_mut()[VIRTIO_NET_HDR_LEN..VIRTIO_NET_HDR_LEN + self.packet_len]
    }

    pub fn virtio_net_header(&self) -> VirtioNetHdr {
        VirtioNetHdr::from_bytes(&self.buf.as_slice()[..VIRTIO_NET_HDR_LEN])
    }
}

/// Buffer for transmit packet
pub struct TxBuffer {
    buf: OwnedFrames,
    buf_len: usize,
}

impl TxBuffer {
    pub fn with_len(buf_len: usize) -> Self {
        let num_pages = buf_len.align_up(PAGE_SIZE) / PAGE_SIZE;
        Self {
            buf: OwnedFrames::new(num_pages).unwrap(),
            buf_len,
        }
    }

    pub fn new(buf: &[u8]) -> Self {
        let buffer = Self::with_len(buf.len());
        buffer.buf.write_bytes(0, buf).unwrap();
        buffer
    }

    pub fn buf(&self) -> &[u8] {
        &self.buf.as_slice()[..self.buf_len]
    }

    pub fn buf_mut(&mut self) -> &mut [u8] {
        &mut self.buf.as_slice_mut()[..self.buf_len]
    }
}
