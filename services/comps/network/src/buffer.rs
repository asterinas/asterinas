// SPDX-License-Identifier: MPL-2.0

use core::mem::size_of;

use align_ext::AlignExt;
use bytes::BytesMut;
use pod::Pod;

/// Buffer for receive packet
#[derive(Debug)]
pub struct RxBuffer {
    /// Packet Buffer, length align 8.
    buf: BytesMut,
    /// Header len
    header_len: usize,
    /// Packet len
    packet_len: usize,
}

impl RxBuffer {
    pub fn new(len: usize, header_len: usize) -> Self {
        let len = len.align_up(8);
        let buf = BytesMut::zeroed(len);
        Self {
            buf,
            packet_len: 0,
            header_len,
        }
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
        debug_assert!(self.header_len + self.packet_len <= self.buf.len());
        &self.buf[self.header_len..self.header_len + self.packet_len]
    }

    /// Mutable packet payload slice.
    pub fn packet_mut(&mut self) -> &mut [u8] {
        debug_assert!(self.header_len + self.packet_len <= self.buf.len());
        &mut self.buf[self.header_len..self.header_len + self.packet_len]
    }

    pub fn header<H: Pod>(&self) -> H {
        debug_assert_eq!(size_of::<H>(), self.header_len);
        H::from_bytes(&self.buf[..size_of::<H>()])
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
