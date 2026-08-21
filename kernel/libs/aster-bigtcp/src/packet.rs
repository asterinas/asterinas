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

use ostd::{
    Result,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasSize, Infallible, PAGE_SIZE, USegment, VmReader,
        VmWriter,
        dma::{DmaStream, ToDevice},
        io::util::HasVmReaderWriter,
    },
};

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

/// A DMA handle.
///
/// This is associated with a [`TxPacket`] and [`RxPacket`] and serves for two purposes:
///  - Enable DMA operations, see [`TxDmaHandle`] and [`RxDmaHandle`].
///  - Keep the buffer alive until [`Self::on_drop`] (which usually means the buffer is managed by a
///    DMA pool).
pub trait DmaHandle: HasDaddr + Send + Sync + ToDmaHandle + 'static {
    /// Releases the DMA buffer back to a DMA allocator.
    ///
    /// `_alloc_start` must be the exact offset returned by a previous [`TxDmaAllocator::alloc`] or
    /// [`RxDmaAllocator::alloc`]. Otherwise, the behavior is unspecified.
    fn on_drop(self: Arc<Self>, _alloc_start: usize) {}
}

#[doc(hidden)]
pub trait ToDmaHandle {
    fn to_dma_handle(self: Arc<Self>) -> Arc<dyn DmaHandle>;
}

impl<T: DmaHandle> ToDmaHandle for T {
    fn to_dma_handle(self: Arc<Self>) -> Arc<dyn DmaHandle> {
        self as _
    }
}

/// A DMA handle that maps a [`TxPacket`].
pub trait TxDmaHandle: DmaHandle {
    /// Synchronizes the DMA mapping data to the device.
    ///
    /// This method should behave in the same way as [`DmaStream::sync_to_device`].
    fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()>;
}

/// A DMA handle that maps a [`RxBuffer`].
pub trait RxDmaHandle: DmaHandle {
    /// Synchronizes the DMA mapping data to the device.
    ///
    /// This method should behave in the same way as [`DmaStream::sync_from_device`].
    fn sync_from_device(&self, byte_range: Range<usize>) -> Result<()>;
}

/// A DMA allocator (which is usually a DMA pool) that allocates [`TxDmaHandle`].
pub trait TxDmaAllocator {
    /// Allocates a [`TxDmaHandle`].
    ///
    /// This method returns an allocation offset. Only the part of the segment starting from that
    /// offset and up to a length of at most `nbytes` should be accessed. The exact offset value
    /// must be passed back to [`DmaHandle::on_drop`] on resource release.
    fn alloc(self: &Arc<Self>, nbytes: usize) -> Result<(USegment, Arc<dyn TxDmaHandle>, usize)>;
}

/// A DMA allocator (which is usually a DMA pool) that allocates [`RxDmaHandle`].
pub trait RxDmaAllocator {
    /// Allocates a [`RxDmaHandle`].
    ///
    /// This method returns an allocation offset. Only the part of the segment starting from that
    /// offset and up to a length of at most `nbytes` should be accessed. The exact offset value
    /// must be passed back to [`DmaHandle::on_drop`] on resource release.
    fn alloc(self: &Arc<Self>, nbytes: usize) -> Result<(USegment, Arc<dyn RxDmaHandle>, usize)>;
}

impl DmaHandle for DmaStream<ToDevice> {}

impl TxDmaHandle for DmaStream<ToDevice> {
    fn sync_to_device(&self, byte_range: Range<usize>) -> Result<()> {
        DmaStream::<ToDevice>::sync_to_device(self, byte_range)
    }
}

struct Common<D: DmaHandle + ?Sized> {
    segment: USegment,
    alloc_range: Range<usize>,
    data_range: Range<usize>,
    dma_handle: Option<Arc<D>>,
}

type TxCommon = Common<dyn TxDmaHandle>;
type RxCommon = Common<dyn RxDmaHandle>;
type AnyCommon = Common<dyn DmaHandle>;

impl<D: DmaHandle + ?Sized> Common<D> {
    fn erase_dma_direction(&mut self) -> Box<AnyCommon> {
        let segment = {
            // Reuse the old segment by replacing it with an empty one.
            let mut segment = self.segment.slice(&(0..0));
            core::mem::swap(&mut self.segment, &mut segment);
            segment
        };

        // Ideally, we should also reuse the old `Box`. However, it is unclear how to do so.
        Box::new(Common {
            segment,
            alloc_range: core::mem::take(&mut self.alloc_range),
            data_range: core::mem::take(&mut self.data_range),
            dma_handle: self.dma_handle.take().map(ToDmaHandle::to_dma_handle),
        })
    }
}

impl<D: DmaHandle + ?Sized> Drop for Common<D> {
    fn drop(&mut self) {
        if let Some(dma_handle) = self.dma_handle.take() {
            dma_handle.on_drop(self.alloc_range.start);
        }
    }
}

/// A fresh, empty TX packet that has just been allocated.
pub struct AllocatedTxPacket<L = ApplicationLayer>(Box<TxCommon>, PhantomData<L>);

impl<L: Layer> AllocatedTxPacket<L> {
    /// Allocates a TX packet.
    ///
    /// The TX packet is not associated with a [`DmaHandle`], but it can acquire one later via
    /// [`TxPacket::map_dma`].
    pub fn new(payload_len: usize) -> Result<Self> {
        Self::new_inner(payload_len + L::HEAD_ROOM_SIZE, L::HEAD_ROOM_SIZE)
    }

    fn new_inner(total_len: usize, head_room_size: usize) -> Result<Self> {
        let nframes = total_len.div_ceil(PAGE_SIZE);
        let segment = FrameAllocOptions::new().alloc_segment(nframes)?.into();

        let common = Common {
            segment,
            alloc_range: 0..total_len,
            data_range: head_room_size..head_room_size,
            dma_handle: None,
        };

        Ok(Self(Box::new(common), PhantomData))
    }

    /// Allocates a TX packet from a DMA allocator.
    ///
    /// The TX packet is associated with a [`DmaHandle`].
    pub fn with_dma<A: TxDmaAllocator>(payload_len: usize, allocator: &Arc<A>) -> Result<Self> {
        Self::with_dma_inner(
            payload_len + L::HEAD_ROOM_SIZE,
            L::HEAD_ROOM_SIZE,
            allocator,
        )
    }

    fn with_dma_inner<A: TxDmaAllocator>(
        total_len: usize,
        head_room_size: usize,
        allocator: &Arc<A>,
    ) -> Result<Self> {
        let (segment, dma_handle, alloc_start) = allocator.alloc(total_len)?;

        let common = Common {
            segment,
            alloc_range: alloc_start..alloc_start + total_len,
            data_range: alloc_start + head_room_size..alloc_start + head_room_size,
            dma_handle: Some(dma_handle),
        };

        Ok(Self(Box::new(common), PhantomData))
    }

    /// Converts to a builder and starts filling the payload.
    pub fn to_builder(self) -> TxPacketBuilder<L> {
        self.to_builder_layer()
    }

    /// Converts to a builder at the specific layer and starts filling the payload.
    pub fn to_builder_layer<H: Layer>(self) -> TxPacketBuilder<H> {
        const { assert!(L::HEAD_ROOM_SIZE >= H::HEAD_ROOM_SIZE) };

        TxPacketBuilder(self.0, PhantomData)
    }
}

/// A TX packet builder that fills some payload into the packet.
pub struct TxPacketBuilder<L>(Box<TxCommon>, PhantomData<L>);

impl<L> TxPacketBuilder<L> {
    /// Returns a writer that can append the payload to the end of the packet.
    pub fn append(&mut self) -> VmWriter<'_, Infallible> {
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
/// - Allocation: [`AllocatedTxPacket::new`] and [`AllocatedTxPacket::with_dma`].
/// - Filling the payload: [`TxPacketBuilder::append`] and [`TxPacketBuilder::commit`].
/// - Filling the headers: [`TxPacket::prepend`] and [`TxPacket::pack`].
/// - Ensuring DMA mapping: [`TxPacket::map_dma`] and [`TxPacket::prepare_dma`].
/// - Device transmission: [`RxBuffer`], which implements [`HasDaddr`].
pub struct TxPacket<L>(Box<TxCommon>, PhantomData<L>);

impl<L> TxPacket<L> {
    /// Returns a reader, starting at the `L` layer.
    pub fn reader(&self) -> VmReader<'_, Infallible> {
        self.reader_with_header(0)
    }

    /// Returns a reader, starting at `header_len` bytes **before** the `L` layer.
    ///
    /// Callers should have used [`Self::prepend`] to fill at least `header_len` bytes before the
    /// `L` layer.
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

    /// Returns a writer that can prepend the header at `header_len` bytes before the `L` layer.
    ///
    /// # Panics
    ///
    /// This method will panic if `header_len` exceeds the allocated header size.
    pub fn prepend(&mut self, header_len: usize) -> VmWriter<'_, Infallible> {
        assert!(header_len <= self.0.data_range.start - self.0.alloc_range.start);
        let mut writer = self.0.segment.writer();
        writer
            .skip(self.0.data_range.start - header_len)
            .limit(header_len);
        writer
    }

    /// Resets the packet back to a fresh, empty allocated TX packet.
    ///
    /// # Panics
    ///
    /// This method will panic if the allocated size is less than the default value of the allocated
    /// header size, which is [`ApplicationLayer::HEAD_ROOM_SIZE`].
    pub fn reset_to_allocated(mut self) -> AllocatedTxPacket {
        const HEAD_ROOM_SIZE: usize = ApplicationLayer::HEAD_ROOM_SIZE;

        assert!(self.0.alloc_range.end - self.0.alloc_range.start >= HEAD_ROOM_SIZE);
        self.0.data_range.start = self.0.alloc_range.start + HEAD_ROOM_SIZE;
        self.0.data_range.end = self.0.alloc_range.start + HEAD_ROOM_SIZE;

        AllocatedTxPacket(self.0, PhantomData)
    }

    /// Converts to an RX packet.
    pub fn to_rx_packet(mut self) -> RxPacket<L> {
        let common = self.0.erase_dma_direction();
        RxPacket(common, PhantomData)
    }
}

impl<L: Layer> TxPacket<L> {
    /// Goes down to the `L::Preceding` layer, assuming that a header whose length is `header_len`
    /// has been filled.
    ///
    /// # Pancis
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
    /// Ensures that the DMA mapping is up to date regarding the new bytes in the packet.
    ///
    /// If the TX packet is not associated with a [`DmaHandle`], this method will attempt to create
    /// one and associate the packet with it.
    pub fn map_dma(mut self, is_cache_coherent: bool) -> Result<TxBuffer> {
        if self.0.dma_handle.is_none() {
            let dma_stream = DmaStream::<ToDevice>::map(self.0.segment.clone(), is_cache_coherent)?;
            let dma_handle: Arc<dyn TxDmaHandle> = Arc::new(dma_stream);
            self.0.dma_handle = Some(dma_handle);
        }

        let tx_buffer = self.prepare_dma().unwrap();
        Ok(tx_buffer)
    }

    /// Tries to ensure that the DMA mapping is up to date regarding the new bytes in the packet.
    ///
    /// If the TX packet is not associated with a [`DmaHandle`] (i.e., if it was oringinally
    /// allocated via [`AllocatedTxPacket::new`] instead [`AllocatedTxPacket::with_dma`]), this
    /// method will return `None`. The method should only be used if the caller knows that the case
    /// won't happen.
    pub fn prepare_dma(self) -> Option<TxBuffer> {
        self.0
            .dma_handle
            .as_ref()?
            .sync_to_device(self.0.data_range.clone())
            .unwrap();
        Some(TxBuffer(self.0))
    }
}

/// A TX buffer, which is a DMA-mapped TX packet.
pub struct TxBuffer(Box<TxCommon>);

impl HasDaddr for TxBuffer {
    fn daddr(&self) -> Daddr {
        self.0.dma_handle.as_ref().unwrap().daddr() + self.0.data_range.start
    }
}

impl HasSize for TxBuffer {
    fn size(&self) -> usize {
        self.0.data_range.end - self.0.data_range.start
    }
}

/// An RX buffer, which is DMA-mapped and can be converted to an RX packet later.
pub struct RxBuffer(RxCommon);

impl RxBuffer {
    /// Allocates an RX buffer from a DMA allocator.
    pub fn alloc<A: RxDmaAllocator>(payload_len: usize, allocator: &Arc<A>) -> Result<Self> {
        let (segment, dma_handle, alloc_start) = allocator.alloc(payload_len)?;

        let common = Common {
            segment,
            alloc_range: alloc_start..alloc_start + payload_len,
            data_range: alloc_start..alloc_start,
            dma_handle: Some(dma_handle),
        };

        Ok(Self(common))
    }

    /// Converts to an RX packet, assuming that `payload_len` bytes have been filled via DMA.
    ///
    /// # Panics
    ///
    /// This method will panic if `payload_len` exceeds the allocated size.
    pub fn finish_dma(self, payload_len: usize) -> RxPacket<DeviceLayer> {
        self.finish_dma_layer(payload_len)
    }

    /// Converts to an RX packet at the specific layer, assuming that `payload_len` bytes have been
    /// filled via DMA.
    ///
    /// # Panics
    ///
    /// This method will panic if `payload_len` exceeds the allocated size.
    pub fn finish_dma_layer<L: Layer<Preceding = ()>>(mut self, payload_len: usize) -> RxPacket<L> {
        assert!(payload_len <= self.0.alloc_range.end - self.0.alloc_range.start);
        self.0.data_range.end = self.0.alloc_range.start + payload_len;

        debug_assert_eq!(self.0.data_range.start, self.0.alloc_range.start);

        self.0
            .dma_handle
            .as_ref()
            .unwrap()
            .sync_from_device(self.0.data_range.clone())
            .unwrap();

        let common = self.0.erase_dma_direction();
        RxPacket(common, PhantomData)
    }
}

impl HasDaddr for RxBuffer {
    fn daddr(&self) -> Daddr {
        self.0.dma_handle.as_ref().unwrap().daddr() + self.0.alloc_range.start
    }
}

impl HasSize for RxBuffer {
    fn size(&self) -> usize {
        self.0.alloc_range.end - self.0.alloc_range.start
    }
}

/// An RX packet.
///
/// The life cycle of a RX packet contains the following phases:
/// - Allocation: [`RxBuffer::alloc`].
/// - Device reception: [`RxBuffer`], which implements [`HasDaddr`], then [`RxBuffer::finish_dma`].
/// - Consuming the headers: [`RxPacket::reader`] and [`RxPacket::peel`].
/// - Consuming the payload: [`RxPacket::reader`].
pub struct RxPacket<L>(Box<AnyCommon>, PhantomData<L>);

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
    /// This method will panic if the number of bytes at the `L` layer is less than `header_len`.
    pub fn peel<H: Layer<Preceding = L>>(mut self, header_len: usize) -> RxPacket<H> {
        debug_assert!(header_len <= L::MAX_HEADER_SIZE);

        assert!(header_len <= self.0.data_range.end - self.0.data_range.start);
        self.0.data_range.start += header_len;

        RxPacket(self.0, PhantomData)
    }
}
