use align_ext::AlignExt;
use bytes::BytesMut;
use pod::Pod;

use crate::device::network::header::VIRTIO_NET_HDR_LEN;

use super::header::VirtioNetHdr;

/// Buffer for receive packet
#[derive(Debug)]
pub struct RxBuffer {
    /// Packet Buffer, length align 8.
    buf: BytesMut,
    /// Packet len
    packet_len: usize,
}

impl RxBuffer {
    pub fn new(len: usize) -> Self {
        let len = len.align_up(8);
        let buf = BytesMut::zeroed(len);
        Self { buf, packet_len: 0 }
    }

    pub const fn packet_len(&self) -> usize {
        self.packet_len
    }

    pub fn set_packet_len(&mut self, packet_len: usize) {
        self.packet_len = packet_len;
    }

    pub fn buf(&self) -> &[u8] {
        &self.buf
    }

    pub fn buf_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }

    /// Packet payload slice, which is inner buffer excluding VirtioNetHdr.
    pub fn packet(&self) -> &[u8] {
        debug_assert!(VIRTIO_NET_HDR_LEN + self.packet_len <= self.buf.len());
        &self.buf[VIRTIO_NET_HDR_LEN..VIRTIO_NET_HDR_LEN + self.packet_len]
    }

    /// Mutable packet payload slice.
    pub fn packet_mut(&mut self) -> &mut [u8] {
        debug_assert!(VIRTIO_NET_HDR_LEN + self.packet_len <= self.buf.len());
        &mut self.buf[VIRTIO_NET_HDR_LEN..VIRTIO_NET_HDR_LEN + self.packet_len]
    }

    pub fn virtio_net_header(&self) -> VirtioNetHdr {
        VirtioNetHdr::from_bytes(&self.buf[..VIRTIO_NET_HDR_LEN])
    }
}

/// Buffer for transmit packet
#[derive(Debug)]
pub struct TxBuffer {
    buf: BytesMut,
}

impl TxBuffer {
    pub fn with_len(buf_len: usize) -> Self {
        Self {
            buf: BytesMut::zeroed(buf_len),
        }
    }

    pub fn new(buf: &[u8]) -> Self {
        Self {
            buf: BytesMut::from(buf),
        }
    }

    pub fn buf(&self) -> &[u8] {
        &self.buf
    }

    pub fn buf_mut(&mut self) -> &mut [u8] {
        &mut self.buf
    }
}
