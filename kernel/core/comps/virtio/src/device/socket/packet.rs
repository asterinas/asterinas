// SPDX-License-Identifier: MPL-2.0

//! DMA-backed virtio-vsock packets.

use aster_bigtcp::packet;
use ostd::{
    Result,
    mm::{Infallible, VmReader, VmWriter},
};

use crate::device::socket::{
    buffer::{RX_BUFFER_LEN, RX_BUFFER_POOL, TX_BUFFER_LEN, TX_BUFFER_POOL},
    header::VirtioVsockHdr,
};

/// An outbound virtio-vsock packet.
pub struct TxPacket(packet::TxBuffer);

impl TxPacket {
    /// Creates a header-only packet carrying `header`.
    pub fn new(header: &VirtioVsockHdr) -> Result<Self> {
        Ok(Self::new_builder()?.build(header))
    }

    /// Creates a builder to build a packet with payload.
    pub fn new_builder() -> Result<TxPacketBuilder> {
        let inner = packet::AllocatedTxPacket::<PayloadLayer>::with_dma(
            TxPacketBuilder::MAX_NBYTES,
            TX_BUFFER_POOL.get().unwrap(),
        )?
        .to_builder();
        Ok(TxPacketBuilder(inner))
    }

    pub(super) fn inner(&self) -> &packet::TxBuffer {
        &self.0
    }
}

/// A builder that builds a [`TxPacket`] with payload before the header is finalized.
pub struct TxPacketBuilder(packet::TxPacketBuilder<PayloadLayer>);

impl TxPacketBuilder {
    /// The maximum payload bytes that fit in one TX packet.
    pub const MAX_NBYTES: usize = TX_BUFFER_LEN - size_of::<VirtioVsockHdr>();

    /// Copies payload bytes to the packet via `copy_fn`.
    pub fn copy_payload<F>(&mut self, copy_fn: F) -> Result<usize>
    where
        F: FnOnce(VmWriter<Infallible>) -> Result<usize>,
    {
        let writer = self.0.append();
        let bytes_written = copy_fn(writer)?;
        self.0.commit(bytes_written);
        Ok(bytes_written)
    }

    /// Returns the payload length accumulated so far.
    pub fn payload_len(&self) -> usize {
        self.0.len()
    }

    /// Finalizes the packet with `header`.
    pub fn build(self, header: &VirtioVsockHdr) -> TxPacket {
        let tx_packet = {
            let mut packet = self.0.build();
            packet
                .prepend(size_of::<VirtioVsockHdr>())
                .write_val(header)
                .unwrap();
            packet.pack(size_of::<VirtioVsockHdr>())
        };

        let tx_buffer = tx_packet.prepare_dma().unwrap();
        TxPacket(tx_buffer)
    }
}

/// An inbound virtio-vsock packet.
pub struct RxPacket(packet::RxPacket<HeaderLayer>);

impl RxPacket {
    pub(super) fn new_builder() -> Result<RxPacketBuilder> {
        let inner = packet::RxBuffer::alloc(RX_BUFFER_LEN, RX_BUFFER_POOL.get().unwrap())?;
        Ok(RxPacketBuilder(inner))
    }

    /// Returns the decoded packet header.
    pub fn header(&self) -> VirtioVsockHdr {
        self.0.reader().read_val::<VirtioVsockHdr>().unwrap()
    }

    /// Returns the payload length in bytes.
    pub fn payload_len(&self) -> usize {
        self.0.len() - size_of::<VirtioVsockHdr>()
    }

    /// Returns a reader over the packet payload.
    pub fn payload(&self) -> VmReader<'_, Infallible> {
        let mut reader = self.0.reader();
        reader.skip(size_of::<VirtioVsockHdr>());
        reader
    }
}

pub(super) struct RxPacketBuilder(packet::RxBuffer);

impl RxPacketBuilder {
    pub(super) fn inner(&self) -> &packet::RxBuffer {
        &self.0
    }

    pub(super) fn build(self, len: usize) -> RxPacket {
        RxPacket(self.0.finish_dma_layer(len))
    }
}

enum HeaderLayer {}
enum PayloadLayer {}

impl packet::Layer for HeaderLayer {
    type Preceding = ();
    const MAX_HEADER_SIZE: usize = size_of::<VirtioVsockHdr>();
}

impl packet::Layer for PayloadLayer {
    type Preceding = HeaderLayer;
    const MAX_HEADER_SIZE: usize = 0;
}
