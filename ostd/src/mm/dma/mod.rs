// SPDX-License-Identifier: MPL-2.0

//! Direct Memory Access (DMA) memory management.
//!
//! This module provides [`DmaCoherent`] and [`DmaStream`] abstractions for
//! managing DMA memory regions with different remapping, caching and
//! synchronization requirements.

#[cfg(ktest)]
mod test;

use core::{any::TypeId, fmt::Debug, marker::PhantomData, mem::ManuallyDrop, ops::Range};

use super::{Daddr, Paddr};
use crate::{
    arch::{
        iommu::{self, has_dma_remapping},
        mm::can_sync_dma,
    },
    cpu::{AtomicCpuSet, CpuSet},
    error::Error,
    mm::{
        io_util::{HasVmReaderWriter, VmReaderWriterResult},
        kspace::kvirt_area::KVirtArea,
        paddr_to_vaddr,
        page_table::vaddr_range,
        tlb::{TlbFlushOp, TlbFlusher},
        CachePolicy, FrameAllocOptions, HasDaddr, HasPaddr, HasPaddrRange, HasSize, Infallible,
        PageFlags, PageProperty, PrivilegedPageFlags, Split, USegment, VmReader, VmWriter,
        PAGE_SIZE,
    },
    task::disable_preempt,
    util::range_alloc::RangeAllocator,
};
#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
use crate::{sync::SpinLock, util::range_counter::RangeCounter};

/// [`DmaDirection`] limits the data flow direction of [`DmaStream`] and
/// prevents users from reading and writing to [`DmaStream`] unexpectedly.
pub trait DmaDirection: 'static + Debug {}

/// Data flows to the device.
///
/// From the perspective of the kernel, this memory region is writable.
#[derive(Debug)]
pub enum ToDevice {}

impl DmaDirection for ToDevice {}

/// Data flows from the device.
///
/// From the perspective of the kernel, this memory region is read-only.
#[derive(Debug)]
pub enum FromDevice {}

impl DmaDirection for FromDevice {}

/// Data flows both from and to the device.
#[derive(Debug)]
pub enum Bidirectional {}

impl DmaDirection for Bidirectional {}

/// A DMA memory object with coherent cache.
pub type DmaCoherent<D = Bidirectional> = Dma<false, D>;

/// A DMA memory object with streaming access.
///
/// The kernel must synchronize the data by [`sync`] when interacting with the
/// device.
///
/// [`sync`]: Dma::sync
pub type DmaStream<D = Bidirectional> = Dma<true, D>;

/// A DMA memory object.
///
/// The `SHOULD_SYNC` generic parameter indicates whether the DMA memory
/// requires synchronization by calling the [`Dma::sync`] method.
///
/// Prefer using the type aliases [`DmaCoherent`] and [`DmaStream`].
#[derive(Debug)]
pub struct Dma<const SHOULD_SYNC: bool, D: DmaDirection> {
    inner: ManuallyDrop<Inner>,
    is_cache_coherent: bool,
    /// If we had DMA remapping enabled, this is the start address of the
    /// DMA memory object in the device address space.
    ///
    /// Otherwise the devices directly uses physical addresses.
    map_daddr: Option<Daddr>,
    _phantom: PhantomData<D>,
}

#[derive(Debug)]
enum Inner {
    /// We access the DMA memory through the segment.
    ///
    /// In this case, we access the DMA memory via the linear mapping.
    Segment(USegment),
    /// We access the DMA memory through a [`KVirtArea`].
    ///
    /// In this case, the kernel allocates and maps a kernel virtual area
    /// to the physical memory address with uncacheable attributes, or shared
    /// attributes in the context of confidential VMs.
    ///
    /// The physical address is mapped to start of the kernel virtual area.
    Kva(KVirtArea, Paddr),
    /// We firstly modify the segment buffer and secondly synchronize DMA
    /// by copying to/from a [`KVirtArea`].
    ///
    /// The DMA physical address is mapped to start of the kernel virtual area,
    /// and might be different from the physical address of the segment buffer.
    Both(KVirtArea, Paddr, USegment),
}

/// The allocator for device addresses.
// TODO: Implement other architectures when their `IommuPtConfig` are ready.
#[cfg(target_arch = "x86_64")]
static DADDR_ALLOCATOR: RangeAllocator = RangeAllocator::new({
    let range_inclusive = vaddr_range::<iommu::IommuPtConfig>();
    // To avoid overflowing, just ignore the last page.
    *range_inclusive.start()..*range_inclusive.end() & !(PAGE_SIZE - 1)
});

/// This is either to
///  - check if the same physical page is DMA mapped twice, or to
///  - track if we need to protect/unprotect pages in the CVM.
#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
static PADDR_REF_CNTS: SpinLock<RangeCounter> = SpinLock::new(RangeCounter::new());

impl<D: DmaDirection> DmaCoherent<D> {
    /// Allocates a region of physical memory for coherent DMA access.
    ///
    /// If the device can access the memory with coherent access to the CPU
    /// cache, set `is_cache_coherent` to `true`.
    pub fn alloc(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        let segment = FrameAllocOptions::new().alloc_segment(nframes)?.into();

        Ok(Self::map_inner(segment, is_cache_coherent))
    }
}

impl<D: DmaDirection> DmaStream<D> {
    /// Establishes DMA stream mapping for a given [`USegment`].
    ///
    /// If the device can access the memory with coherent access to the CPU
    /// cache, set `is_cache_coherent` to `true`.
    pub fn map(segment: USegment, is_cache_coherent: bool) -> Self {
        Self::map_inner(segment, is_cache_coherent)
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> Dma<SHOULD_SYNC, D> {
    fn map_inner(segment: USegment, is_cache_coherent: bool) -> Self {
        #[cfg(target_arch = "x86_64")]
        let has_tdx = crate::arch::if_tdx_enabled!({ true } else { false });
        #[cfg(not(target_arch = "x86_64"))]
        let has_tdx = false;

        let needed_inner_type =
            Self::determine_inner_type(is_cache_coherent, has_tdx, can_sync_dma());

        let inner = match needed_inner_type {
            InnerType::Segment => Inner::Segment(segment),
            InnerType::Kva(cache)
            | InnerType::SegmentKvaSharedPaddr(cache)
            | InnerType::SegmentKvaDistinctPaddr(cache) => {
                #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
                let priv_flags = if has_tdx {
                    PrivilegedPageFlags::SHARED
                } else {
                    PrivilegedPageFlags::empty()
                };
                #[cfg(not(all(target_arch = "x86_64", feature = "cvm_guest")))]
                let priv_flags = PrivilegedPageFlags::empty();

                let (mapped_seg, orig_seg) = match needed_inner_type {
                    InnerType::SegmentKvaSharedPaddr(_) => (segment.clone(), Some(segment)),
                    InnerType::SegmentKvaDistinctPaddr(_) => {
                        let allocated = FrameAllocOptions::new()
                            .alloc_segment(segment.size() / PAGE_SIZE)
                            .expect("Failed to allocate distinct segment for DMA stream")
                            .into();
                        (allocated, Some(segment))
                    }
                    _ => (segment, None),
                };

                let paddr = mapped_seg.paddr();
                let kva = KVirtArea::map_frames(
                    mapped_seg.size(),
                    0,
                    mapped_seg,
                    PageProperty {
                        flags: PageFlags::RW,
                        cache,
                        priv_flags,
                    },
                );

                let target_cpus = AtomicCpuSet::new(CpuSet::new_full());
                let mut flusher = TlbFlusher::new(&target_cpus, disable_preempt());
                flusher.issue_tlb_flush(TlbFlushOp::for_range(kva.range()));
                flusher.dispatch_tlb_flush();
                flusher.sync_tlb_flush();

                match needed_inner_type {
                    InnerType::Kva(_) => Inner::Kva(kva, paddr),
                    _ => Inner::Both(kva, paddr, orig_seg.unwrap()),
                }
            }
        };

        // Check for overlapping DMA mappings in TDX guest or debug builds.
        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        Self::check_or_map_new_physical_range(inner.paddr_range());

        let map_daddr = Self::dma_remap(inner.paddr_range());

        Self {
            inner: ManuallyDrop::new(inner),
            is_cache_coherent,
            map_daddr,
            _phantom: PhantomData,
        }
    }

    #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
    fn check_or_map_new_physical_range(pa_range: Range<Paddr>) {
        use alloc::{vec, vec::Vec};

        let ranges = PADDR_REF_CNTS.lock().add(&pa_range);
        #[cfg(target_arch = "x86_64")]
        crate::arch::if_tdx_enabled!({
            for partial in ranges {
                debug_assert_eq!(partial, pa_range);
                // SAFETY:
                //  - The provided physical address is page aligned.
                //  - The provided physical address range is in the limit.
                //  - All of the physical pages are untyped memory.
                unsafe {
                    crate::arch::tdx_guest::unprotect_gpa_tdvm_call(
                        partial.start,
                        partial.end - partial.start,
                    )
                    .expect("Failed to protect the DMA segment in TDX guest");
                }
            }
        } else {
            debug_assert_eq!(ranges.collect::<Vec<_>>(), vec![pa_range]);
        });
        #[cfg(not(target_arch = "x86_64"))]
        debug_assert_eq!(ranges.collect::<Vec<_>>(), vec![pa_range]);
    }

    fn dma_remap(pa_range: Range<Paddr>) -> Option<Daddr> {
        if has_dma_remapping() {
            #[cfg(target_arch = "x86_64")]
            let daddr = DADDR_ALLOCATOR
                .alloc(pa_range.len())
                .expect("Failed to allocate DMA address range");
            #[cfg(not(target_arch = "x86_64"))]
            let daddr = pa_range.clone();

            for map_paddr in pa_range.clone().step_by(PAGE_SIZE) {
                let map_daddr = (map_paddr - pa_range.start + daddr.start) as Daddr;
                // SAFETY: the `map_daddr` and `map_paddr` are both valid.
                unsafe {
                    iommu::map(map_daddr, map_paddr).unwrap();
                }
            }
            Some(daddr.start)
        } else {
            None
        }
    }

    const fn determine_inner_type(
        is_cache_coherent: bool,
        tdx_enabled: bool,
        has_cache_maintenance_instrs: bool,
    ) -> InnerType {
        if tdx_enabled {
            let cache = if is_cache_coherent {
                CachePolicy::Writeback
            } else {
                CachePolicy::Uncacheable
            };
            if SHOULD_SYNC {
                // After `DmaStream::map`, the caller can still access the
                // segment through private mapping. So we map a newly
                // allocated segment and sync through copying.
                InnerType::SegmentKvaDistinctPaddr(cache)
            } else {
                // `DmaCoherent::alloc`. We need a shared mapping for TDX. So
                // we map a KVA regardless of cache coherence.
                InnerType::Kva(cache)
            }
        } else if is_cache_coherent {
            InnerType::Segment
        } else if SHOULD_SYNC {
            // `DmaStream::map`.
            if has_cache_maintenance_instrs {
                InnerType::Segment
            } else {
                InnerType::SegmentKvaSharedPaddr(CachePolicy::Writeback)
            }
        } else {
            // `DmaCoherent::alloc`.
            InnerType::Kva(CachePolicy::Uncacheable)
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum InnerType {
    Segment,
    Kva(CachePolicy),
    SegmentKvaSharedPaddr(CachePolicy),
    SegmentKvaDistinctPaddr(CachePolicy),
}

impl<D: DmaDirection> DmaStream<D> {
    /// Synchronizes the streaming DMA mapping with the device.
    ///
    /// This method should be called under one of the two conditions:
    ///  1. The data of the stream DMA mapping has been updated by the device
    ///     side. And the CPU side needs to call the `sync` method before
    ///     reading data (e.g., using [`read_bytes`]).
    ///  2. The data of the stream DMA mapping has been updated by the CPU side
    ///     (e.g., using [`write_bytes`]). Before the CPU side notifies the
    ///     device side to read, it must call the `sync` method first.
    ///
    /// [`read_bytes`]: crate::mm::VmIo::read_bytes
    /// [`write_bytes`]: crate::mm::VmIo::write_bytes
    pub fn sync(&self, byte_range: Range<usize>) -> Result<(), Error> {
        let size = self.size();
        if byte_range.end > size || byte_range.start > size {
            return Err(Error::InvalidArgs);
        }
        if self.is_cache_coherent {
            return Ok(());
        }

        let va_range = match &*self.inner {
            Inner::Segment(segment) => {
                let pa_range = segment.paddr_range();
                paddr_to_vaddr(pa_range.start)..paddr_to_vaddr(pa_range.end)
            }
            Inner::Kva(kva, _) => kva.range(),
            Inner::Both(kva, _, seg) => {
                // In this case we synchronize through copying.
                let skip = byte_range.start;
                let limit = byte_range.len();
                // SAFETY: The area is fully mapped with untyped memory.
                let mut kva_writer =
                    unsafe { VmWriter::from_kernel_space(kva.start() as *mut u8, kva.size()) };
                kva_writer
                    .skip(skip)
                    .limit(limit)
                    .write(seg.reader().skip(skip).limit(limit));
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
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> Split for Dma<SHOULD_SYNC, D> {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset % PAGE_SIZE == 0);
        assert!(0 < offset && offset < self.size());

        let mut old = ManuallyDrop::new(self);

        // SAFETY: The old value will never be used again.
        let (a1, a2) = match unsafe { ManuallyDrop::take(&mut old.inner) } {
            Inner::Segment(segment) => {
                let (s1, s2) = segment.split(offset);
                (Inner::Segment(s1), Inner::Segment(s2))
            }
            Inner::Kva(kva, paddr) => {
                let (kva1, kva2) = kva.split(offset);
                let paddr1 = paddr;
                let paddr2 = paddr + offset;
                (Inner::Kva(kva1, paddr1), Inner::Kva(kva2, paddr2))
            }
            Inner::Both(kva, paddr, segment) => {
                let (kva1, kva2) = kva.split(offset);
                let paddr1 = paddr;
                let paddr2 = paddr + offset;
                let (s1, s2) = segment.split(offset);
                (Inner::Both(kva1, paddr1, s1), Inner::Both(kva2, paddr2, s2))
            }
        };

        let (daddr1, daddr2) = match old.map_daddr {
            Some(daddr) => {
                let daddr1 = daddr;
                let daddr2 = daddr + offset;
                (Some(daddr1), Some(daddr2))
            }
            None => (None, None),
        };

        let is_cache_coherent = old.is_cache_coherent;

        (
            Self {
                inner: ManuallyDrop::new(a1),
                is_cache_coherent,
                map_daddr: daddr1,
                _phantom: PhantomData,
            },
            Self {
                inner: ManuallyDrop::new(a2),
                is_cache_coherent,
                map_daddr: daddr2,
                _phantom: PhantomData,
            },
        )
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> Drop for Dma<SHOULD_SYNC, D> {
    fn drop(&mut self) {
        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        let _removed_frames = PADDR_REF_CNTS.lock().remove(&self.paddr_range());
        match self.map_daddr {
            None => {
                #[cfg(target_arch = "x86_64")]
                crate::arch::if_tdx_enabled!({
                    for pa_range in _removed_frames {
                        // SAFETY: The physical address range is unprotected
                        // before and valid to protect. No race because of
                        // reference counting.
                        unsafe {
                            crate::arch::tdx_guest::protect_gpa_tdvm_call(
                                pa_range.start,
                                pa_range.end - pa_range.start,
                            )
                            .expect("Failed to protect the DMA segment in TDX guest");
                        }
                    }
                });
            }
            Some(daddr) => {
                let frame_count = self.size() / PAGE_SIZE;
                for i in 0..frame_count {
                    let map_daddr = daddr + (i * PAGE_SIZE);
                    iommu::unmap(map_daddr).unwrap();
                    // FIXME: After dropping it could be reused. IOTLB needs to be flushed.
                }
            }
        }
        // SAFETY: We're dropping the `Dma`, so the `inner` will never
        // be used again.
        unsafe { ManuallyDrop::drop(&mut self.inner) };
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasDaddr for Dma<SHOULD_SYNC, D> {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasPaddr for Dma<SHOULD_SYNC, D> {
    fn paddr(&self) -> Paddr {
        self.inner.paddr()
    }
}

impl HasPaddr for Inner {
    fn paddr(&self) -> Paddr {
        match self {
            Inner::Segment(segment) => segment.paddr(),
            Inner::Kva(_, paddr) => *paddr,
            Inner::Both(_, paddr, _) => *paddr, // the mapped PA, not the buffer's PA
        }
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasSize for Dma<SHOULD_SYNC, D> {
    fn size(&self) -> usize {
        self.inner.size()
    }
}

impl HasSize for Inner {
    fn size(&self) -> usize {
        match self {
            Inner::Segment(segment) => segment.size(),
            Inner::Kva(kva, _) => kva.size(),
            Inner::Both(kva, _, segment) => {
                debug_assert_eq!(segment.size(), kva.size());
                segment.size()
            }
        }
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasVmReaderWriter for Dma<SHOULD_SYNC, D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        if TypeId::of::<D>() == TypeId::of::<ToDevice>() {
            return Err(Error::AccessDenied);
        }
        match &*self.inner {
            Inner::Segment(seg) | Inner::Both(_, _, seg) => Ok(seg.reader()),
            Inner::Kva(kva, _) => Ok(
                // SAFETY: The area is fully mapped with untyped memory.
                unsafe { VmReader::from_kernel_space(kva.start() as *const u8, kva.size()) },
            ),
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        if TypeId::of::<D>() == TypeId::of::<FromDevice>() {
            return Err(Error::AccessDenied);
        }
        match &*self.inner {
            Inner::Segment(seg) | Inner::Both(_, _, seg) => Ok(seg.writer()),
            Inner::Kva(kva, _) => Ok(
                // SAFETY: The area is fully mapped with untyped memory.
                unsafe { VmWriter::from_kernel_space(kva.start() as *mut u8, kva.size()) },
            ),
        }
    }
}
