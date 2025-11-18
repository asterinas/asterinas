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
    arch::iommu::{self, has_dma_remapping},
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

/// A DMA mappped memory object.
#[derive(Debug)]
pub struct Dma<const SHOULD_SYNC: bool, D: DmaDirection> {
    kernel_access: ManuallyDrop<DmaKernelAccess>,
    is_cache_coherent: bool,
    /// If we had DMA remapping enabled, this is the start address of the
    /// DMA memory object in the device address space.
    ///
    /// Otherwise the devices directly uses physical addresses.
    map_daddr: Option<Daddr>,
    _phantom: PhantomData<D>,
}

/// The way kernel accesses the DMA memory object.
#[derive(Debug)]
enum DmaKernelAccess {
    /// The kernel accesses the DMA memory object through a cached mapping.
    ///
    /// In this case, the kernel holds the ownership of the physical memory
    /// segment and access via the linear mapping.
    Cached(USegment),
    /// The kernel accesses the DMA memory object through an uncached mapping.
    ///
    /// In this case, the kernel allocates and maps a kernel virtual area
    /// to the physical memory address with uncacheable attributes.
    ///
    /// The physical address is also cached here to avoid querying over the
    /// page table.
    Uncached(KVirtArea, Paddr),
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
        let paddr = segment.paddr();
        let size = segment.size();

        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        #[cfg_attr(
            not(all(target_arch = "x86_64", feature = "cvm_guest")),
            expect(unused)
        )]
        let newly_added_frames = {
            let ranges = PADDR_REF_CNTS
                .lock()
                .add(&(paddr..paddr + size))
                .collect::<alloc::vec::Vec<_>>();
            debug_assert_eq!(
                ranges.first().cloned(),
                Some(paddr..paddr + size),
                "Some frames are mapped twice as DMA memory"
            );
            ranges
        };

        let kernel_access = if !SHOULD_SYNC && !is_cache_coherent {
            // The user neither wants to sync the data nor the device can access the memory
            // coherently, so we must use uncached mappings.
            #[cfg(target_arch = "x86_64")]
            let priv_flags = crate::arch::if_tdx_enabled!({
                PrivilegedPageFlags::SHARED
            } else {
                PrivilegedPageFlags::empty()
            });
            #[cfg(not(target_arch = "x86_64"))]
            let priv_flags = { PrivilegedPageFlags::empty() };

            let kva = KVirtArea::map_frames(
                segment.size(),
                0,
                segment,
                PageProperty {
                    flags: PageFlags::RW,
                    cache: CachePolicy::Uncacheable,
                    priv_flags,
                },
            );

            let target_cpus = AtomicCpuSet::new(CpuSet::new_full());
            let mut flusher = TlbFlusher::new(&target_cpus, disable_preempt());
            flusher.issue_tlb_flush(TlbFlushOp::for_range(kva.range()));
            flusher.dispatch_tlb_flush();
            flusher.sync_tlb_flush();

            DmaKernelAccess::Uncached(kva, paddr)
        } else {
            // The user wants to sync the data or the device can access the memory coherently,
            // so we can use cached mappings.
            DmaKernelAccess::Cached(segment)
        };

        let map_daddr = if has_dma_remapping() {
            #[cfg(target_arch = "x86_64")]
            let daddr = DADDR_ALLOCATOR
                .alloc(size)
                .expect("Failed to allocate DMA address range");
            #[cfg(not(target_arch = "x86_64"))]
            let daddr = paddr..paddr + size;

            for i in (0..size).step_by(PAGE_SIZE) {
                let map_daddr = (daddr.start + i) as Daddr;
                let map_paddr = paddr + i;
                // SAFETY: the `map_daddr` and `map_paddr` are both valid.
                unsafe {
                    iommu::map(map_daddr, map_paddr).unwrap();
                }
            }
            Some(daddr.start)
        } else {
            #[cfg(target_arch = "x86_64")]
            crate::arch::if_tdx_enabled!({
                for pa_range in newly_added_frames {
                    // SAFETY:
                    //  - The provided physical address is page aligned.
                    //  - The provided physical address range is in the limit.
                    //  - All of the physical pages are untyped memory.
                    unsafe {
                        crate::arch::tdx_guest::unprotect_gpa_tdvm_call(
                            pa_range.start,
                            pa_range.end - pa_range.start,
                        )
                        .expect("Failed to protect the DMA segment in TDX guest");
                    }
                }
            });
            None
        };

        Self {
            kernel_access: ManuallyDrop::new(kernel_access),
            is_cache_coherent,
            map_daddr,
            _phantom: PhantomData,
        }
    }
}

impl<D: DmaDirection> Dma<true, D> {
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

        let va_range = match &*self.kernel_access {
            DmaKernelAccess::Cached(segment) => {
                let pa_range = segment.paddr_range();
                paddr_to_vaddr(pa_range.start)..paddr_to_vaddr(pa_range.end)
            }
            DmaKernelAccess::Uncached(kva, _) => kva.range(),
        };
        // SAFETY: We've checked that the range is inbound, so the virtual
        // address range and the DMA direction correspond to a DMA region
        // (they're part of `self`).
        unsafe { crate::arch::mm::sync_dma_range::<D>(va_range) };

        Ok(())
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> Split for Dma<SHOULD_SYNC, D> {
    fn split(self, offset: usize) -> (Self, Self) {
        assert!(offset % PAGE_SIZE == 0);
        assert!(0 < offset && offset < self.size());

        let mut old = ManuallyDrop::new(self);

        // SAFETY: The old value will never be used again.
        let (a1, a2) = match unsafe { ManuallyDrop::take(&mut old.kernel_access) } {
            DmaKernelAccess::Cached(segment) => {
                let (s1, s2) = segment.split(offset);
                (DmaKernelAccess::Cached(s1), DmaKernelAccess::Cached(s2))
            }
            DmaKernelAccess::Uncached(kva, paddr) => {
                let (kva1, kva2) = kva.split(offset);
                let paddr1 = paddr;
                let paddr2 = paddr + offset;
                (
                    DmaKernelAccess::Uncached(kva1, paddr1),
                    DmaKernelAccess::Uncached(kva2, paddr2),
                )
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
                kernel_access: ManuallyDrop::new(a1),
                is_cache_coherent,
                map_daddr: daddr1,
                _phantom: PhantomData,
            },
            Self {
                kernel_access: ManuallyDrop::new(a2),
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
        #[cfg_attr(
            not(all(target_arch = "x86_64", feature = "cvm_guest")),
            expect(unused)
        )]
        let removed_frames = PADDR_REF_CNTS.lock().remove(&self.paddr_range());
        match self.map_daddr {
            None => {
                #[cfg(target_arch = "x86_64")]
                crate::arch::if_tdx_enabled!({
                    for pa_range in removed_frames {
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
        // SAFETY: We're dropping the `Dma`, so the `kernel_access` will never
        // be used again.
        unsafe { ManuallyDrop::drop(&mut self.kernel_access) };
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasDaddr for Dma<SHOULD_SYNC, D> {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasPaddr for Dma<SHOULD_SYNC, D> {
    fn paddr(&self) -> Paddr {
        match &*self.kernel_access {
            DmaKernelAccess::Cached(segment) => segment.paddr(),
            DmaKernelAccess::Uncached(_, paddr) => *paddr,
        }
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasSize for Dma<SHOULD_SYNC, D> {
    fn size(&self) -> usize {
        match &*self.kernel_access {
            DmaKernelAccess::Cached(segment) => segment.size(),
            DmaKernelAccess::Uncached(kva, _) => kva.size(),
        }
    }
}

impl<const SHOULD_SYNC: bool, D: DmaDirection> HasVmReaderWriter for Dma<SHOULD_SYNC, D> {
    type Types = VmReaderWriterResult;

    fn reader(&self) -> Result<VmReader<'_, Infallible>, Error> {
        if TypeId::of::<D>() == TypeId::of::<ToDevice>() {
            return Err(Error::AccessDenied);
        }
        match &*self.kernel_access {
            DmaKernelAccess::Cached(segment) => Ok(segment.reader()),
            DmaKernelAccess::Uncached(kva, _) => Ok(
                // SAFETY:
                //  - The memory range points to untyped memory.
                //  - The frame/segment is alive during the lifetime `'_`.
                //  - Using `VmReader` and `VmWriter` is the only way to access the frame/segment.
                unsafe { VmReader::from_kernel_space(kva.start() as *const u8, kva.size()) },
            ),
        }
    }

    fn writer(&self) -> Result<VmWriter<'_, Infallible>, Error> {
        if TypeId::of::<D>() == TypeId::of::<FromDevice>() {
            return Err(Error::AccessDenied);
        }
        match &*self.kernel_access {
            DmaKernelAccess::Cached(segment) => Ok(segment.writer()),
            DmaKernelAccess::Uncached(kva, _) => Ok(
                // SAFETY: We ensure that only untyped memory are mapped into the area.
                unsafe { VmWriter::from_kernel_space(kva.start() as *mut u8, kva.size()) },
            ),
        }
    }
}
