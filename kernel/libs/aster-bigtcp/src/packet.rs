// SPDX-License-Identifier: MPL-2.0

//! Network packet abstractions.
//!
//! Network packets consist of headers from multiple layers and the actual payload. When sending or
//! receiving a packet, the headers are added or removed as the packet goes through the network
//! stack.
//!
//! We use [`TxPacket`] and [`RxPacket`], plus a marker that specifies the layer ([`DeviceLayer`],
//! [`LinkLayer`], [`NetworkLayer`], [`TransportLayer`], and [`ApplicationLayer`]), to denote a
//! packet that is being processed by that that network layer.
//!
//! Here is an illustration of `TxPacket<TransportLayer>` (the part above) and
//! `RxPacket<NetworkLayer>` (the part below):
//!
//! ```text
//!                                   TxPacket<TransportLayer>
//!                                             |
//! <--------- Headers (to be filled) --------> | <------- Payload (filled) ---------->
//!                                             |
//!                                             v
//! +--------------+------------+---------------+-----------------+-------------------+
//! | Device Layer | Link Layer | Network Layer | Transport Layer | Application Layer |
//! +--------------+------------+---------------+-----------------+-------------------+
//!                             ^
//!                             |
//! <--- Headers (consumed) --> | <----------- Payload (to be consumed) -------------->
//!                             |
//!                         RxPacket<NetworkLayer>
//! ```
//!

use alloc::{boxed::Box, sync::Arc};
use core::{marker::PhantomData, ops::Range};

use dma_pool::{DmaBuffer, DmaPool};
use ostd::{
    Result,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasSize, Infallible, PAGE_SIZE, USegment, VmReader,
        VmWriter,
        dma::{DmaDirection, DmaStream, FromDevice, ToDevice},
        io::util::HasVmReaderWriter,
    },
};
use ostd_pod::Pod;

/// A network layer.
pub trait Layer {
    /// The network layer preceding to this layer.
    ///
    /// This should be specified as the unit type `()` for the first layer with no preceding layers.
    type Preceding: Layer;

    /// The maximum header size for all devices/protocols that we support in this layer.
    ///
    /// Note that the value should be adjusted when supporting new devices/protocols that require
    /// large headers.
    const MAX_HEADER_SIZE: usize;

    /// The sum of the [`Layer::MAX_HEADER_SIZE`] for all layers preceding this one.
    const HEAD_ROOM_SIZE: usize =
        Self::Preceding::MAX_HEADER_SIZE + Self::Preceding::HEAD_ROOM_SIZE;
}

impl Layer for () {
    type Preceding = ();

    const MAX_HEADER_SIZE: usize = 0;
    const HEAD_ROOM_SIZE: usize = 0;
}

pub enum DeviceLayer {}
pub enum LinkLayer {}
pub enum NetworkLayer {}
pub enum TransportLayer {}
pub enum ApplicationLayer {}

impl Layer for DeviceLayer {
    type Preceding = ();
    // The size of a virtio-net header.
    const MAX_HEADER_SIZE: usize = 12;
}

impl Layer for LinkLayer {
    type Preceding = DeviceLayer;
    // The size of an Ethernet header.
    const MAX_HEADER_SIZE: usize = 14;
}

impl Layer for NetworkLayer {
    type Preceding = LinkLayer;
    // The maximum size of an IPv4 header.
    const MAX_HEADER_SIZE: usize = 60;
}

impl Layer for TransportLayer {
    type Preceding = NetworkLayer;
    // The maximum size of a TCP header.
    const MAX_HEADER_SIZE: usize = 60;
}

impl Layer for ApplicationLayer {
    type Preceding = TransportLayer;
    // There are no headers.
    const MAX_HEADER_SIZE: usize = 0;
}

struct Common<D: DmaDirection> {
    segment: USegment,
    /// The range of allocation, starting from the beginning of `segment`.
    alloc_range: Range<usize>,
    /// The range of data, starting from the beginning of `segment`.
    data_range: Range<usize>,
    dma_buffer: Option<DmaBuffer<D>>,
}

impl<D: DmaDirection> Common<D> {
    /// Returns a range to perform [`DmaBuffer::sync_to_device`] or [`DmaBuffer::sync_from_device`].
    ///
    /// Note that this is a range starting from the beginning of allocation (instead of `segment`).
    fn range_to_sync(&self) -> Range<usize> {
        let offset = self.alloc_range.start;
        self.data_range.start - offset..self.data_range.end - offset
    }
}

type TxCommon = Common<ToDevice>;
type RxCommon = Common<FromDevice>;

/// A fresh, empty TX packet that has just been allocated.
pub struct FreshTxPacket<L = ApplicationLayer>(Box<TxCommon>, PhantomData<L>);

impl<L: Layer> FreshTxPacket<L> {
    /// Allocates a TX packet.
    ///
    /// The TX packet is not associated with a DMA buffer, but it can acquire one later via
    /// [`TxPacket::map_dma`].
    pub fn alloc(payload_len: usize) -> Result<Self> {
        Self::alloc_inner(payload_len + L::HEAD_ROOM_SIZE, L::HEAD_ROOM_SIZE)
    }

    fn alloc_inner(total_len: usize, head_room_size: usize) -> Result<Self> {
        let nframes = total_len.div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new().alloc_segment(nframes)?.into();

        let common = Common {
            segment,
            alloc_range: 0..total_len,
            data_range: head_room_size..head_room_size,
            dma_buffer: None,
        };

        Ok(Self(Box::new(common), PhantomData))
    }

    /// Allocates a TX packet from a DMA pool.
    ///
    /// The TX packet is associated with a DMA buffer.
    ///
    /// # Panics
    ///
    /// This method will panic if the size of a segment allocated from the DMA pool is less than the
    /// header size to allocate (see [`Layer::HEAD_ROOM_SIZE`]).
    pub fn alloc_from_pool(pool: &Arc<DmaPool<ToDevice>>) -> Result<Self> {
        Self::alloc_from_pool_inner(L::HEAD_ROOM_SIZE, pool)
    }

    fn alloc_from_pool_inner(head_room_size: usize, pool: &Arc<DmaPool<ToDevice>>) -> Result<Self> {
        assert!(pool.segment_size() > head_room_size);

        let dma_segment = pool.alloc_segment()?;
        let (segment, alloc_range) = dma_segment.storage().into_parts();

        let common = Common {
            segment,
            alloc_range: alloc_range.clone(),
            data_range: alloc_range.start + head_room_size..alloc_range.start + head_room_size,
            dma_buffer: Some(DmaBuffer::Pooled(dma_segment)),
        };

        Ok(Self(Box::new(common), PhantomData))
    }

    /// Converts to a builder and starts filling the payload.
    pub fn to_builder(self) -> TxPacketBuilder<L> {
        self.to_builder_at_layer()
    }

    /// Converts to a builder at the specific layer and starts filling the payload.
    pub fn to_builder_at_layer<H: Layer>(self) -> TxPacketBuilder<H> {
        const { assert!(L::HEAD_ROOM_SIZE >= H::HEAD_ROOM_SIZE) };

        TxPacketBuilder(self.0, PhantomData)
    }
}

/// A TX packet builder that fills some payload into the packet.
pub struct TxPacketBuilder<L>(Box<TxCommon>, PhantomData<L>);

impl<L> TxPacketBuilder<L> {
    /// Returns a writer that can append the payload to the end of the packet.
    pub fn append_writer(&mut self) -> VmWriter<'_, Infallible> {
        let mut writer = self.0.segment.writer();
        writer
            .skip(self.0.data_range.end)
            .limit(self.0.alloc_range.end - self.0.data_range.end);
        writer
    }

    /// Commits the payload with the given length.
    ///
    /// # Panics
    ///
    /// This method will panic if the total payload length after the operation exceeds the allocated
    /// payload size.
    pub fn commit(&mut self, payload_len: usize) {
        assert!(payload_len <= self.0.alloc_range.end - self.0.data_range.end);
        self.0.data_range.end += payload_len;
    }

    /// Returns the length of the payload.
    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }

    /// Finishes the payload and starts filling the headers.
    pub fn build(self) -> TxPacket<L> {
        TxPacket(self.0, PhantomData)
    }
}

/// A TX packet.
///
/// The life cycle of a TX packet contains the following phases:
/// - Allocation: [`FreshTxPacket::alloc`] and [`FreshTxPacket::alloc_from_pool`].
/// - Filling the payload: [`TxPacketBuilder::append_writer`] and [`TxPacketBuilder::commit`].
/// - Filling the headers: [`TxPacket::prepend_writer`] and [`TxPacket::pack`].
/// - Creating DMA buffer: [`TxPacket::map_dma`] and [`TxPacket::to_dma_buffer`].
/// - Device transmission: [`RxBuffer`], which implements [`HasDaddr`].
pub struct TxPacket<L>(Box<TxCommon>, PhantomData<L>);

impl<L> TxPacket<L> {
    /// Returns a reader, starting at the `L` layer.
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        self.reader_with_header(0)
    }

    /// Returns a reader, starting at `header_len` bytes **before** the `L` layer.
    ///
    /// Callers should have used [`Self::prepend_writer`] to fill at least `header_len` bytes before
    /// the `L` layer.
    ///
    /// # Panics
    ///
    /// This method will panic if `header_len` exceeds the allocated header size.
    pub fn reader_with_header(&self, header_len: usize) -> VmReader<'_, Infallible> {
        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        let mut reader = self.0.segment.reader();
        reader
            .skip(self.0.data_range.start - header_len)
            .limit(self.0.data_range.end - self.0.data_range.start + header_len);
        reader
    }

    /// Returns the number of bytes at the `L` layer.
    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }

    /// Returns a writer that can prepend a header at `header_len` bytes before the `L` layer.
    ///
    /// # Panics
    ///
    /// This method will panic if `header_len` exceeds the allocated header size.
    pub fn prepend_writer(&mut self, header_len: usize) -> VmWriter<'_, Infallible> {
        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        let mut writer = self.0.segment.writer();
        writer
            .skip(self.0.data_range.start - header_len)
            .limit(header_len);
        writer
    }

    /// Resets the packet back to a fresh, empty TX packet.
    ///
    /// # Panics
    ///
    /// This method will panic if the allocated size is less than the default value of the allocated
    /// header size, which is [`ApplicationLayer::HEAD_ROOM_SIZE`].
    pub fn reset_to_fresh(mut self) -> FreshTxPacket {
        const HEAD_ROOM_SIZE: usize = ApplicationLayer::HEAD_ROOM_SIZE;

        assert!(self.0.alloc_range.end - self.0.alloc_range.start >= HEAD_ROOM_SIZE);
        self.0.data_range.start = self.0.alloc_range.start + HEAD_ROOM_SIZE;
        self.0.data_range.end = self.0.alloc_range.start + HEAD_ROOM_SIZE;

        FreshTxPacket(self.0, PhantomData)
    }

    /// Converts to an RX packet.
    ///
    /// If the TX packet is associated with a DMA buffer (i.e., if it was oringinally allocated via
    /// [`FreshTxPacket::alloc_from_pool`] instead [`FreshTxPacket::alloc`]), this method will
    /// return `None`.
    pub fn to_rx_packet(self) -> Option<RxPacket<L>> {
        if self.0.dma_buffer.is_some() {
            return None;
        }

        let common = Box::map(self.0, |common| Common {
            segment: common.segment,
            alloc_range: common.alloc_range,
            data_range: common.data_range,
            dma_buffer: None,
        });
        Some(RxPacket(common, PhantomData))
    }
}

impl<L: Layer> TxPacket<L> {
    /// Prepends a header before the `L` layer and goes down to the `L::Preceding` layer.
    ///
    /// This operation is equivalent to writing the header to [`Self::prepend_writer`] and then
    /// adjusting the layer with [`Self::pack`].
    ///
    /// # Panics
    ///
    /// This method will panic if `size_of::<T>()` exceeds the allocated header size.
    pub fn prepend_and_pack<T: Pod>(mut self, header: &T) -> TxPacket<L::Preceding> {
        self.prepend_writer(size_of::<T>())
            .write_val(header)
            .unwrap();
        self.pack(size_of::<T>())
    }

    /// Goes down to the `L::Preceding` layer, assuming that a header whose length is `header_len`
    /// has been filled.
    ///
    /// # Panics
    ///
    /// This method will panic if the total header length after the operation exceeds the allocated
    /// header size.
    pub fn pack(mut self, header_len: usize) -> TxPacket<L::Preceding> {
        debug_assert!(header_len <= L::Preceding::MAX_HEADER_SIZE);

        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        self.0.data_range.start -= header_len;

        TxPacket(self.0, PhantomData)
    }
}

impl<L: Layer<Preceding = ()>> TxPacket<L> {
    /// Ensures that the DMA buffer is up to date regarding the new bytes in the packet.
    ///
    /// If the TX packet is not associated with a DMA buffer, this method will attempt to create one
    /// and associate the packet with it.
    pub fn map_dma(mut self, is_cache_coherent: bool) -> Result<TxBuffer> {
        if self.0.dma_buffer.is_none() {
            let dma_stream = DmaStream::<ToDevice>::map(self.0.segment.clone(), is_cache_coherent)?;
            self.0.dma_buffer = Some(DmaBuffer::Direct(dma_stream));
        }

        let tx_buffer = self.to_dma_buffer().unwrap();
        Ok(tx_buffer)
    }

    /// Tries to ensure that the DMA buffer is up to date regarding the new bytes in the packet.
    ///
    /// If the TX packet is not associated with a DMA buffer (i.e., if it was oringinally allocated
    /// via [`FreshTxPacket::alloc`] instead [`FreshTxPacket::alloc_from_pool`]), this method will
    /// return `None`.
    pub fn to_dma_buffer(self) -> Option<TxBuffer> {
        self.0
            .dma_buffer
            .as_ref()?
            .sync_to_device(self.0.range_to_sync())
            .unwrap();
        Some(TxBuffer(self.0))
    }
}

/// A TX buffer, which is a DMA-mapped TX packet.
pub struct TxBuffer(Box<TxCommon>);

impl HasDaddr for TxBuffer {
    fn daddr(&self) -> Daddr {
        let offset = self.0.data_range.start - self.0.alloc_range.start;
        self.0.dma_buffer.as_ref().unwrap().daddr() + offset
    }
}

impl HasSize for TxBuffer {
    fn size(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }
}

/// An RX buffer, which is DMA-mapped and can be converted to an RX packet later.
pub struct RxBuffer(Box<RxCommon>);

impl RxBuffer {
    /// Allocates an RX buffer from a DMA pool.
    pub fn alloc(pool: &Arc<DmaPool<FromDevice>>) -> Result<Self> {
        let dma_segment = pool.alloc_segment()?;
        let (segment, alloc_range) = dma_segment.storage().into_parts();

        let common = Common {
            segment,
            alloc_range: alloc_range.clone(),
            data_range: alloc_range.start..alloc_range.start,
            dma_buffer: Some(DmaBuffer::Pooled(dma_segment)),
        };

        Ok(Self(Box::new(common)))
    }

    /// Converts to an RX packet, assuming that `payload_len` bytes have been filled via DMA.
    ///
    /// # Panics
    ///
    /// This method will panic if `payload_len` exceeds the allocated size.
    pub fn finish_dma(self, payload_len: usize) -> RxPacket<DeviceLayer> {
        self.finish_dma_at_layer(payload_len)
    }

    /// Converts to an RX packet at the specific layer, assuming that `payload_len` bytes have been
    /// filled via DMA.
    ///
    /// # Panics
    ///
    /// This method will panic if `payload_len` exceeds the allocated size.
    pub fn finish_dma_at_layer<L: Layer<Preceding = ()>>(
        mut self,
        payload_len: usize,
    ) -> RxPacket<L> {
        assert!(payload_len <= self.0.alloc_range.end - self.0.alloc_range.start);
        self.0.data_range.end = self.0.alloc_range.start + payload_len;

        debug_assert_eq!(self.0.data_range.start, self.0.alloc_range.start);

        self.0
            .dma_buffer
            .as_ref()
            .unwrap()
            .sync_from_device(self.0.range_to_sync())
            .unwrap();

        RxPacket(self.0, PhantomData)
    }
}

impl HasDaddr for RxBuffer {
    fn daddr(&self) -> Daddr {
        self.0.dma_buffer.as_ref().unwrap().daddr()
    }
}

impl HasSize for RxBuffer {
    fn size(&self) -> usize {
        self.0.alloc_range.end - self.0.alloc_range.start
    }
}

/// An RX packet.
///
/// The life cycle of an RX packet contains the following phases:
/// - Allocation: [`RxBuffer::alloc`].
/// - Device reception: [`RxBuffer`], which implements [`HasDaddr`], then [`RxBuffer::finish_dma`].
/// - Consuming the headers: [`RxPacket::reader`] and [`RxPacket::peel`].
/// - Consuming the payload: [`RxPacket::reader`].
pub struct RxPacket<L>(Box<RxCommon>, PhantomData<L>);

impl<L> RxPacket<L> {
    /// Returns a reader, starting at the `L` layer.
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        self.reader_with_header(0)
    }

    /// Returns a reader, starting at `header_len` bytes **before** the `L` layer.
    ///
    /// # Panics
    ///
    /// This method will panic if `header_len` exceeds the allocated header size.
    pub fn reader_with_header(&self, header_len: usize) -> VmReader<'_, Infallible> {
        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        let mut reader = self.0.segment.reader();
        reader
            .skip(self.0.data_range.start - header_len)
            .limit(self.0.data_range.end - self.0.data_range.start + header_len);
        reader
    }

    /// Returns the number of bytes at the `L` layer.
    #[expect(clippy::len_without_is_empty)]
    pub fn len(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }

    /// Truncates the number of bytes at the `L` layter to `len`.
    ///
    /// # Panics
    ///
    /// This method will panic if `len` exceeds the number of bytes at the `L` layer.
    pub fn truncate(&mut self, len: usize) {
        assert!(len <= self.0.data_range.end - self.0.data_range.start);
        self.0.data_range.end = self.0.data_range.start + len;
    }
}

impl<L: Layer> RxPacket<L> {
    /// Goes up to the `H` layer, assuming that a header whose length is `header_len` has been
    /// consumed.
    ///
    /// # Panics
    ///
    /// This method will panic if `header_len` exceeds the number of bytes at the `L` layer.
    pub fn peel<H: Layer<Preceding = L>>(mut self, header_len: usize) -> RxPacket<H> {
        debug_assert!(header_len <= L::MAX_HEADER_SIZE);

        assert!(header_len <= self.0.data_range.end - self.0.data_range.start);
        self.0.data_range.start += header_len;

        RxPacket(self.0, PhantomData)
    }
}
