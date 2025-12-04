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
        CachePolicy, FrameAllocOptions, HasDaddr, HasPaddr, HasPaddrRange, HasSize, Infallible,
        PAGE_SIZE, PageFlags, PageProperty, PrivilegedPageFlags, Split, USegment, VmReader,
        VmWriter,
        io_util::{HasVmReaderWriter, VmReaderWriterIdentity, VmReaderWriterResult},
        kspace::kvirt_area::KVirtArea,
        paddr_to_vaddr,
        page_table::vaddr_range,
        tlb::{TlbFlushOp, TlbFlusher},
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
#[derive(Debug)]
pub struct DmaCoherent {
    inner: ManuallyDrop<CoherentInner>,
    map_daddr: Option<Daddr>,
    is_cache_coherent: bool,
}

#[derive(Debug)]
enum CoherentInner {
    Segment(USegment),
    Kva(KVirtArea, Paddr),
}

/// A DMA memory object with streaming access.
///
/// The kernel must synchronize the data by [`sync`] when interacting with the
/// device.
///
/// [`sync`]: DmaStream::sync
#[derive(Debug)]
pub struct DmaStream<D: DmaDirection = Bidirectional> {
    inner: ManuallyDrop<StreamInner>,
    map_daddr: Option<Daddr>,
    is_cache_coherent: bool,
    _phantom: PhantomData<D>,
}

#[derive(Debug)]
enum StreamInner {
    Segment(USegment),
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

impl DmaCoherent {
    /// Allocates a region of physical memory for coherent DMA access.
    ///
    /// If the device can access the memory with coherent access to the CPU
    /// cache, set `is_cache_coherent` to `true`.
    pub fn alloc(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        let segment: USegment = FrameAllocOptions::new().alloc_segment(nframes)?.into();
        let paddr_range = segment.paddr_range();

        let has_tdx = has_tdx();

        let inner = if is_cache_coherent && !has_tdx {
            CoherentInner::Segment(segment)
        } else {
            let (kva, paddr) = map_kva(segment, has_tdx, is_cache_coherent);
            CoherentInner::Kva(kva, paddr)
        };

        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        alloc_unprotect_physical_range(&paddr_range);

        let map_daddr = dma_remap(&paddr_range);

        Ok(Self {
            inner: ManuallyDrop::new(inner),
            map_daddr,
            is_cache_coherent,
        })
    }
}

impl<D: DmaDirection> DmaStream<D> {
    /// Establishes DMA stream mapping for a given [`USegment`].
    ///
    /// If the device can access the memory with coherent access to the CPU
    /// cache, set `is_cache_coherent` to `true`.
    pub fn map(segment: USegment, is_cache_coherent: bool) -> Self {
        let has_tdx = has_tdx();

        let inner = if (can_sync_dma() || is_cache_coherent) && !has_tdx {
            StreamInner::Segment(segment)
        } else {
            let (seg_to_map, orig_seg) = if has_tdx {
                // Allocate another private segment and synchronize through copying.
                let allocated = FrameAllocOptions::new()
                    .alloc_segment(segment.size() / PAGE_SIZE)
                    .expect("Failed to allocate distinct segment for DMA stream")
                    .into();
                (allocated, segment)
            } else {
                (segment.clone(), segment)
            };

            let (kva, paddr) = map_kva(seg_to_map, has_tdx, is_cache_coherent);

            StreamInner::Both(kva, paddr, orig_seg)
        };

        let paddr_range = inner.paddr_range();

        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        alloc_unprotect_physical_range(&paddr_range);

        let map_daddr = dma_remap(&paddr_range);

        Self {
            inner: ManuallyDrop::new(inner),
            map_daddr,
            is_cache_coherent,
            _phantom: PhantomData,
        }
    }

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
            StreamInner::Segment(segment) => {
                let pa_range = segment.paddr_range();
                paddr_to_vaddr(pa_range.start)..paddr_to_vaddr(pa_range.end)
            }
            StreamInner::Both(kva, _, seg) => {
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

impl HasPaddr for StreamInner {
    fn paddr(&self) -> Paddr {
        match self {
            StreamInner::Segment(segment) => segment.paddr(),
            StreamInner::Both(_, paddr, _) => *paddr, // the mapped PA, not the buffer's PA
        }
    }
}

impl HasSize for StreamInner {
    fn size(&self) -> usize {
        match self {
            StreamInner::Segment(segment) => segment.size(),
            StreamInner::Both(kva, _, segment) => {
                debug_assert_eq!(kva.size(), segment.size());
                kva.size()
            }
        }
    }
}

fn has_tdx() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::if_tdx_enabled!({ true } else { false })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

fn map_kva(segment: USegment, has_tdx: bool, is_cache_coherent: bool) -> (KVirtArea, Paddr) {
    #[cfg(all(target_arch = "x86_64", feature = "cvm_guest"))]
    let priv_flags = if has_tdx {
        PrivilegedPageFlags::SHARED
    } else {
        PrivilegedPageFlags::empty()
    };
    #[cfg(not(all(target_arch = "x86_64", feature = "cvm_guest")))]
    let priv_flags = {
        let _ = has_tdx;
        PrivilegedPageFlags::empty()
    };

    let cache = if is_cache_coherent {
        CachePolicy::Writeback
    } else {
        CachePolicy::Uncacheable
    };

    let paddr = segment.paddr();
    let kva = KVirtArea::map_frames(
        segment.size(),
        0,
        segment,
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

    (kva, paddr)
}

#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
fn alloc_unprotect_physical_range(pa_range: &Range<Paddr>) {
    use alloc::{vec, vec::Vec};

    let mut refcnts = PADDR_REF_CNTS.lock();
    let ranges = refcnts.add(pa_range);
    #[cfg(target_arch = "x86_64")]
    crate::arch::if_tdx_enabled!({
        for partial in ranges {
            debug_assert_eq!(partial, pa_range.clone());
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
        debug_assert_eq!(ranges.collect::<Vec<_>>(), vec![pa_range.clone()]);
    });
    #[cfg(not(target_arch = "x86_64"))]
    debug_assert_eq!(ranges.collect::<Vec<_>>(), vec![pa_range.clone()]);
}

#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
fn dealloc_protect_physical_range(pa_range: &Range<Paddr>) {
    let mut refcnts = PADDR_REF_CNTS.lock();
    let _removed_frames = refcnts.remove(pa_range);
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

fn dma_remap(pa_range: &Range<Paddr>) -> Option<Daddr> {
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

fn unmap_dma_remap(daddr_range: Option<Range<Daddr>>) {
    if let Some(da_range) = daddr_range {
        for da in da_range.step_by(PAGE_SIZE) {
            iommu::unmap(da).unwrap();
            // FIXME: After dropping it could be reused. IOTLB needs to be flushed.
        }
    }
}

impl Split for DmaCoherent {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset.is_multiple_of(PAGE_SIZE));
        assert!(0 < offset && offset < self.size());

        let mut old = ManuallyDrop::new(self);

        // SAFETY: The old value will never be used again.
        let (a1, a2) = match unsafe { ManuallyDrop::take(&mut old.inner) } {
            CoherentInner::Segment(segment) => {
                let (s1, s2) = segment.split(offset);
                (CoherentInner::Segment(s1), CoherentInner::Segment(s2))
            }
            CoherentInner::Kva(kva, paddr) => {
                let (kva1, kva2) = kva.split(offset);
                let paddr1 = paddr;
                let paddr2 = paddr + offset;
                (
                    CoherentInner::Kva(kva1, paddr1),
                    CoherentInner::Kva(kva2, paddr2),
                )
            }
        };

        let (daddr1, daddr2) = split_daddr(old.map_daddr, offset);

        let is_cache_coherent = old.is_cache_coherent;
        (
            Self {
                inner: ManuallyDrop::new(a1),
                map_daddr: daddr1,
                is_cache_coherent,
            },
            Self {
                inner: ManuallyDrop::new(a2),
                map_daddr: daddr2,
                is_cache_coherent,
            },
        )
    }
}

impl<D: DmaDirection> Split for DmaStream<D> {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset.is_multiple_of(PAGE_SIZE));
        assert!(0 < offset && offset < self.size());

        let mut old = ManuallyDrop::new(self);

        // SAFETY: The old value will never be used again.
        let (a1, a2) = match unsafe { ManuallyDrop::take(&mut old.inner) } {
            StreamInner::Segment(segment) => {
                let (s1, s2) = segment.split(offset);
                (StreamInner::Segment(s1), StreamInner::Segment(s2))
            }
            StreamInner::Both(kva, paddr, segment) => {
                let (kva1, kva2) = kva.split(offset);
                let paddr1 = paddr;
                let paddr2 = paddr + offset;
                let (s1, s2) = segment.split(offset);
                (
                    StreamInner::Both(kva1, paddr1, s1),
                    StreamInner::Both(kva2, paddr2, s2),
                )
            }
        };

        let (daddr1, daddr2) = split_daddr(old.map_daddr, offset);

        let is_cache_coherent = old.is_cache_coherent;
        (
            Self {
                inner: ManuallyDrop::new(a1),
                map_daddr: daddr1,
                is_cache_coherent,
                _phantom: PhantomData,
            },
            Self {
                inner: ManuallyDrop::new(a2),
                map_daddr: daddr2,
                is_cache_coherent,
                _phantom: PhantomData,
            },
        )
    }
}

fn split_daddr(daddr: Option<Daddr>, offset: usize) -> (Option<Daddr>, Option<Daddr>) {
    match daddr {
        Some(daddr) => {
            let daddr1 = daddr;
            let daddr2 = daddr + offset;
            (Some(daddr1), Some(daddr2))
        }
        None => (None, None),
    }
}

impl Drop for DmaCoherent {
    fn drop(&mut self) {
        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        dealloc_protect_physical_range(&self.paddr_range());

        unmap_dma_remap(self.map_daddr.map(|daddr| daddr..daddr + self.size()));

        // SAFETY: We're dropping the `Dma`, so the `inner` will never
        // be used again.
        unsafe { ManuallyDrop::drop(&mut self.inner) };
    }
}

impl<D: DmaDirection> Drop for DmaStream<D> {
    fn drop(&mut self) {
        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        dealloc_protect_physical_range(&self.paddr_range());

        unmap_dma_remap(self.map_daddr.map(|daddr| daddr..daddr + self.size()));

        // SAFETY: We're dropping the `Dma`, so the `inner` will never
        // be used again.
        unsafe { ManuallyDrop::drop(&mut self.inner) };
    }
}

impl HasDaddr for DmaCoherent {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
    }
}

impl<D: DmaDirection> HasDaddr for DmaStream<D> {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
    }
}

impl HasPaddr for DmaCoherent {
    fn paddr(&self) -> Paddr {
        match &*self.inner {
            CoherentInner::Segment(segment) => segment.paddr(),
            CoherentInner::Kva(_, paddr) => *paddr,
        }
    }
}

impl<D: DmaDirection> HasPaddr for DmaStream<D> {
    fn paddr(&self) -> Paddr {
        self.inner.paddr()
    }
}

impl HasSize for DmaCoherent {
    fn size(&self) -> usize {
        match &*self.inner {
            CoherentInner::Segment(segment) => segment.size(),
            CoherentInner::Kva(kva, _) => kva.size(),
        }
    }
}

impl<D: DmaDirection> HasSize for DmaStream<D> {
    fn size(&self) -> usize {
        self.inner.size()
    }
}

impl HasVmReaderWriter for DmaCoherent {
    type Types = VmReaderWriterIdentity;

    fn reader(&self) -> VmReader<'_, Infallible> {
        match &*self.inner {
            CoherentInner::Segment(seg) => seg.reader(),
            CoherentInner::Kva(kva, _) => {
                // SAFETY: The area is fully mapped with untyped memory.
                unsafe { VmReader::from_kernel_space(kva.start() as *const u8, kva.size()) }
            }
        }
    }

    fn writer(&self) -> VmWriter<'_, Infallible> {
        match &*self.inner {
            CoherentInner::Segment(seg) => seg.writer(),
            CoherentInner::Kva(kva, _) => {
                // SAFETY: The area is fully mapped with untyped memory.
                unsafe { VmWriter::from_kernel_space(kva.start() as *mut u8, kva.size()) }
            }
        }
    }
}

impl<D: DmaDirection> HasVmReaderWriter for DmaStream<D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        if TypeId::of::<D>() == TypeId::of::<ToDevice>() {
            return Err(Error::AccessDenied);
        }
        match &*self.inner {
            StreamInner::Segment(seg) | StreamInner::Both(_, _, seg) => Ok(seg.reader()),
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        if TypeId::of::<D>() == TypeId::of::<FromDevice>() {
            return Err(Error::AccessDenied);
        }
        match &*self.inner {
            StreamInner::Segment(seg) | StreamInner::Both(_, _, seg) => Ok(seg.writer()),
        }
    }
}
