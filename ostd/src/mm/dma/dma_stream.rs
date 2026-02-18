// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Debug, marker::PhantomData, mem::ManuallyDrop, ops::Range};

use super::util::{
    alloc_kva, cvm_need_private_protection, prepare_dma, split_daddr, unprepare_dma,
};
use crate::{
    arch::mm::can_sync_dma,
    error::Error,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasPaddr, HasPaddrRange, HasSize, Infallible,
        PAGE_SIZE, Paddr, Split, USegment, VmReader, VmWriter,
        io_util::{HasVmReaderWriter, VmReaderWriterResult},
        kspace::kvirt_area::KVirtArea,
        paddr_to_vaddr,
    },
};

/// [`DmaDirection`] limits the data flow direction of [`DmaStream`] and
/// prevents users from reading and writing to [`DmaStream`] unexpectedly.
pub trait DmaDirection: 'static + Debug + private::Sealed {
    /// Whether the CPU can read data from the device.
    const CAN_READ_FROM_DEVICE: bool;
    /// Whether the CPU can write data to the device.
    const CAN_WRITE_TO_DEVICE: bool;
}

mod private {
    /// To avoid users implement `DmaDirection` and triggers unreachable code in
    /// functions like [`crate::arch::mm::sync_dma_range`], or bypasses checks
    /// in [`crate::mm::io_util::HasVmReaderWriter`].
    pub trait Sealed {}
}

/// Data flows to the device.
///
/// From the perspective of the kernel, this memory region is writable.
#[derive(Debug)]
pub enum ToDevice {}

impl private::Sealed for ToDevice {}
impl DmaDirection for ToDevice {
    const CAN_READ_FROM_DEVICE: bool = false;
    const CAN_WRITE_TO_DEVICE: bool = true;
}

/// Data flows from the device.
///
/// From the perspective of the kernel, this memory region is read-only.
#[derive(Debug)]
pub enum FromDevice {}

impl private::Sealed for FromDevice {}
impl DmaDirection for FromDevice {
    const CAN_READ_FROM_DEVICE: bool = true;
    const CAN_WRITE_TO_DEVICE: bool = false;
}

/// Data flows both from and to the device.
#[derive(Debug)]
pub enum FromAndToDevice {}

impl private::Sealed for FromAndToDevice {}
impl DmaDirection for FromAndToDevice {
    const CAN_READ_FROM_DEVICE: bool = true;
    const CAN_WRITE_TO_DEVICE: bool = true;
}

/// A DMA memory object with streaming access.
///
/// The kernel must synchronize the data by [`sync_from_device`]/[`sync_to_device`]
/// when interacting with the device.
///
/// [`sync_from_device`]: DmaStream::sync_from_device
/// [`sync_to_device`]: DmaStream::sync_to_device
#[derive(Debug)]
pub struct DmaStream<D: DmaDirection = FromAndToDevice> {
    inner: Inner,
    map_daddr: Option<Daddr>,
    is_cache_coherent: bool,
    _phantom: PhantomData<D>,
}

#[derive(Debug)]
enum Inner {
    Segment(USegment),
    Kva(KVirtArea, Paddr),
    Both(KVirtArea, Paddr, USegment),
}

impl<D: DmaDirection> DmaStream<D> {
    /// Allocates a region of physical memory for streaming DMA access.
    ///
    /// The memory of the newly-allocated DMA buffer is initialized to zeros.
    /// This method is only available when `D` is [`ToDevice`] or
    /// [`FromAndToDevice`], as zeroing requires write access to the buffer.
    ///
    /// The `is_cache_coherent` argument specifies whether the target device
    /// that the DMA mapping is prepared for can access the main memory in a
    /// CPU cache coherent way or not.
    ///
    /// # Comparison with [`DmaStream::map`]
    ///
    /// This method is semantically equivalent to allocating a [`USegment`] via
    /// [`FrameAllocOptions::alloc_segment`] and then mapping it with
    /// [`DmaStream::map`]. However, [`DmaStream::alloc`] combines these two
    /// operations and can be more efficient in certain scenarios, particularly
    /// in confidential VMs, where the overhead of bounce buffers can be
    /// avoided.
    pub fn alloc(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        const { assert!(D::CAN_WRITE_TO_DEVICE) };

        Self::alloc_uninit(nframes, is_cache_coherent).and_then(|dma| {
            dma.writer()?.fill_zeros(dma.size());
            Ok(dma)
        })
    }

    /// Allocates a region of physical memory for streaming DMA access
    /// without initialization.
    ///
    /// This method is the same as [`DmaStream::alloc`]
    /// except that it skips zeroing the memory of newly-allocated DMA region.
    pub fn alloc_uninit(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        let cvm = cvm_need_private_protection();

        let (inner, paddr_range) = if (can_sync_dma() || is_cache_coherent) && !cvm {
            let segment: USegment = FrameAllocOptions::new()
                .zeroed(false)
                .alloc_segment(nframes)?
                .into();
            let paddr_range = segment.paddr_range();

            (Inner::Segment(segment), paddr_range)
        } else {
            let (kva, paddr) = alloc_kva(nframes, can_sync_dma() || is_cache_coherent)?;

            (Inner::Kva(kva, paddr), paddr..paddr + nframes * PAGE_SIZE)
        };

        // SAFETY: The physical address range is untyped DMA memory before `drop`.
        let map_daddr = unsafe { prepare_dma(&paddr_range) };

        Ok(Self {
            inner,
            map_daddr,
            is_cache_coherent,
            _phantom: PhantomData,
        })
    }

    /// Establishes DMA stream mapping for a given [`USegment`].
    ///
    /// The `is_cache_coherent` argument specifies whether the target device
    /// that the DMA mapping is prepared for can access the main memory in a
    /// CPU cache coherent way or not.
    pub fn map(segment: USegment, is_cache_coherent: bool) -> Result<Self, Error> {
        let cvm = cvm_need_private_protection();
        let size = segment.size();

        let (inner, paddr) = if (can_sync_dma() || is_cache_coherent) && !cvm {
            let paddr = segment.paddr();

            (Inner::Segment(segment), paddr)
        } else {
            let (kva, paddr) = alloc_kva(size / PAGE_SIZE, is_cache_coherent)?;

            (Inner::Both(kva, paddr, segment), paddr)
        };

        let paddr_range = paddr..paddr + size;

        // SAFETY: The physical address range is untyped DMA memory before `drop`.
        let map_daddr = unsafe { prepare_dma(&paddr_range) };

        Ok(Self {
            inner,
            map_daddr,
            is_cache_coherent,
            _phantom: PhantomData,
        })
    }

    /// Synchronizes the streaming DMA mapping data from the device.
    ///
    /// This method should be called when the data of the streaming DMA mapping
    /// has been updated by the device side. Before the CPU side starts to read
    /// (e.g., using [`read_bytes`]), it must call the [`Self::sync_from_device`]
    /// method first.
    ///
    /// [`read_bytes`]: crate::mm::VmIo::read_bytes
    pub fn sync_from_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        const { assert!(D::CAN_READ_FROM_DEVICE) };

        self.sync_impl(byte_range, true)
    }

    /// Synchronizes the streaming DMA mapping data to the device.
    ///
    /// This method should be called when the data of the streaming DMA mapping
    /// has been updated by the CPU side (e.g., using [`write_bytes`]). Before
    /// the CPU side notifies the device side to read, it must call the
    /// [`Self::sync_to_device`] method first.
    ///
    /// [`write_bytes`]: crate::mm::VmIo::write_bytes
    pub fn sync_to_device(&self, byte_range: Range<usize>) -> Result<(), Error> {
        const { assert!(D::CAN_WRITE_TO_DEVICE) };

        self.sync_impl(byte_range, false)
    }

    fn sync_impl(&self, byte_range: Range<usize>, is_from_device: bool) -> Result<(), Error> {
        let size = self.size();
        if byte_range.end > size || byte_range.start > size {
            return Err(Error::InvalidArgs);
        }
        if self.is_cache_coherent {
            return Ok(());
        }

        let va_range = match &self.inner {
            Inner::Segment(segment) => {
                let pa_range = segment.paddr_range();
                paddr_to_vaddr(pa_range.start)..paddr_to_vaddr(pa_range.end)
            }
            Inner::Kva(kva, _) => {
                if !can_sync_dma() {
                    // The KVA is mapped as uncachable.
                    return Ok(());
                }
                kva.range()
            }
            Inner::Both(kva, _, seg) => {
                self.sync_via_copying(byte_range, is_from_device, seg, kva);
                return Ok(());
            }
        };
        let range = va_range.start + byte_range.start..va_range.start + byte_range.end;

        // SAFETY: We've checked that the range is inbound, so the virtual
        // address range and the DMA direction correspond to a DMA region
        // (they're part of `self`).
        unsafe { crate::arch::mm::sync_dma_range::<D>(range) };

        Ok(())
    }

    fn sync_via_copying(
        &self,
        byte_range: Range<usize>,
        is_from_device: bool,
        seg: &USegment,
        kva: &KVirtArea,
    ) {
        let skip = byte_range.start;
        let limit = byte_range.len();

        let (mut reader, mut writer) = if is_from_device {
            // SAFETY:
            //  - The memory range points to untyped memory.
            //  - The KVA is alive in this scope.
            //  - Using `VmReader` and `VmWriter` is the only way to access the KVA.
            let kva_reader =
                unsafe { VmReader::from_kernel_space(kva.start() as *const u8, kva.size()) };

            (kva_reader, seg.writer())
        } else {
            // SAFETY:
            //  - The memory range points to untyped memory.
            //  - The KVA is alive in this scope.
            //  - Using `VmReader` and `VmWriter` is the only way to access the KVA.
            let kva_writer =
                unsafe { VmWriter::from_kernel_space(kva.start() as *mut u8, kva.size()) };

            (seg.reader(), kva_writer)
        };

        writer
            .skip(skip)
            .limit(limit)
            .write(reader.skip(skip).limit(limit));
    }
}

impl<D: DmaDirection> Split for DmaStream<D> {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset.is_multiple_of(PAGE_SIZE));
        assert!(0 < offset && offset < self.size());

        let (inner, map_daddr, is_cache_coherent) = {
            let this = ManuallyDrop::new(self);
            (
                // SAFETY: `this.inner` will never be used or dropped later.
                unsafe { core::ptr::read(&this.inner as *const Inner) },
                this.map_daddr,
                this.is_cache_coherent,
            )
        };

        let (inner1, inner2) = match inner {
            Inner::Segment(segment) => {
                let (s1, s2) = segment.split(offset);
                (Inner::Segment(s1), Inner::Segment(s2))
            }
            Inner::Kva(kva, paddr) => {
                let (kva1, kva2) = kva.split(offset);
                let (paddr1, paddr2) = (paddr, paddr + offset);
                (Inner::Kva(kva1, paddr1), Inner::Kva(kva2, paddr2))
            }
            Inner::Both(kva, paddr, segment) => {
                let (kva1, kva2) = kva.split(offset);
                let (paddr1, paddr2) = (paddr, paddr + offset);
                let (s1, s2) = segment.split(offset);
                (Inner::Both(kva1, paddr1, s1), Inner::Both(kva2, paddr2, s2))
            }
        };

        let (daddr1, daddr2) = split_daddr(map_daddr, offset);

        (
            Self {
                inner: inner1,
                map_daddr: daddr1,
                is_cache_coherent,
                _phantom: PhantomData,
            },
            Self {
                inner: inner2,
                map_daddr: daddr2,
                is_cache_coherent,
                _phantom: PhantomData,
            },
        )
    }
}

impl<D: DmaDirection> Drop for DmaStream<D> {
    fn drop(&mut self) {
        // SAFETY: The physical address range was prepared in `map`.
        unsafe { unprepare_dma(&self.paddr_range(), self.map_daddr) };
    }
}

impl<D: DmaDirection> HasPaddr for DmaStream<D> {
    fn paddr(&self) -> Paddr {
        match &self.inner {
            Inner::Segment(segment) => segment.paddr(),
            Inner::Kva(_, paddr) | Inner::Both(_, paddr, _) => *paddr, // the mapped PA, not the buffer's PA
        }
    }
}

impl<D: DmaDirection> HasDaddr for DmaStream<D> {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
    }
}

impl<D: DmaDirection> HasSize for DmaStream<D> {
    fn size(&self) -> usize {
        match &self.inner {
            Inner::Segment(segment) => segment.size(),
            Inner::Kva(kva, _) => kva.size(),
            Inner::Both(kva, _, segment) => {
                debug_assert_eq!(kva.size(), segment.size());
                kva.size()
            }
        }
    }
}

impl<D: DmaDirection> HasVmReaderWriter for DmaStream<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        if !D::CAN_READ_FROM_DEVICE {
            return Err(Error::AccessDenied);
        }
        match &self.inner {
            Inner::Segment(seg) | Inner::Both(_, _, seg) => Ok(seg.reader()),
            Inner::Kva(kva, _) => {
                // SAFETY:
                //  - Although the memory range points to typed memory, the range is for DMA
                //    and the access is not by linear mapping.
                //  - The KVA is alive during the lifetime `'_`.
                //  - Using `VmReader` and `VmWriter` is the only way to access the KVA.
                unsafe {
                    Ok(VmReader::from_kernel_space(
                        kva.start() as *const u8,
                        kva.size(),
                    ))
                }
            }
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        if !D::CAN_WRITE_TO_DEVICE {
            return Err(Error::AccessDenied);
        }
        match &self.inner {
            Inner::Segment(seg) | Inner::Both(_, _, seg) => Ok(seg.writer()),
            Inner::Kva(kva, _) => {
                // SAFETY:
                //  - Although the memory range points to typed memory, the range is for DMA
                //    and the access is not by linear mapping.
                //  - The KVA is alive during the lifetime `'_`.
                //  - Using `VmReader` and `VmWriter` is the only way to access the KVA.
                unsafe {
                    Ok(VmWriter::from_kernel_space(
                        kva.start() as *mut u8,
                        kva.size(),
                    ))
                }
            }
        }
    }
}
