// SPDX-License-Identifier: MPL-2.0

//! DMA-backed virtio-vsock packets.

use aster_network::{RxBuffer, TxBuffer, TxBufferBuilder};
use ostd::{
    Result,
    mm::{Infallible, VmReader, VmWriter},
};

use crate::device::socket::{
    buffer::{RX_BUFFER_POOL, TX_BUFFER_LEN, TX_BUFFER_POOL},
    header::VirtioVsockHdr,
};

/// An outbound virtio-vsock packet.
pub struct TxPacket(TxBuffer);

impl TxPacket {
    /// Creates a header-only packet carrying `header`.
    pub fn new(header: &VirtioVsockHdr) -> Result<Self> {
        Ok(Self::new_builder()?.build(header))
    }

    /// Creates a builder to build a packet with payload.
    pub fn new_builder() -> Result<TxPacketBuilder> {
        TxBuffer::new_builder(TX_BUFFER_POOL.get().unwrap()).map(TxPacketBuilder)
    }

    pub(super) fn inner(&self) -> &TxBuffer {
        &self.0
    }
}

/// A builder that builds a [`TxPacket`] with payload before the header is finalized.
pub struct TxPacketBuilder(TxBufferBuilder<VirtioVsockHdr>);

impl TxPacketBuilder {
    /// The maximum payload bytes that fit in one TX packet.
    pub const MAX_NBYTES: usize = TX_BUFFER_LEN - size_of::<VirtioVsockHdr>();

    /// Copies payload bytes to the packet via `copy_fn`.
    pub fn copy_payload<F>(&mut self, copy_fn: F) -> Result<usize>
    where
        F: FnOnce(VmWriter<Infallible>) -> Result<usize>,
    {
        self.0.copy_payload(copy_fn)
    }

    /// Returns the payload length accumulated so far.
    pub fn payload_len(&self) -> usize {
        self.0.payload_len()
    }

    /// Finalizes the packet with `header`.
    pub fn build(self, header: &VirtioVsockHdr) -> TxPacket {
        TxPacket(self.0.build(header))
    }
}

/// An inbound virtio-vsock packet.
pub struct RxPacket(RxBuffer);

impl RxPacket {
    pub(super) fn new() -> Result<Self> {
        RxBuffer::new(size_of::<VirtioVsockHdr>(), RX_BUFFER_POOL.get().unwrap()).map(Self)
    }

    pub(super) fn set_payload_len(&mut self, len: usize) {
        self.0.set_payload_len(len);
    }

    pub(super) fn inner(&self) -> &RxBuffer {
        &self.0
    }

    /// Returns the decoded packet header.
    pub fn header(&self) -> VirtioVsockHdr {
        self.0.buf().read_val::<VirtioVsockHdr>().unwrap()
    }

    /// Returns the payload length in bytes.
    pub fn payload_len(&self) -> usize {
        self.0.payload_len()
    }

    /// Returns a reader over the packet payload.
    pub fn payload(&self) -> VmReader<'_, Infallible> {
        self.0.payload()
    }
}
