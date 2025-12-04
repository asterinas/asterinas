// SPDX-License-Identifier: MPL-2.0

use core::ops::Range;

use crate::{
    Error,
    arch::iommu::{self, has_dma_remapping},
    cpu::{AtomicCpuSet, CpuSet},
    impl_frame_meta_for,
    mm::{
        CachePolicy, Daddr, FrameAllocOptions, HasPaddr, HasSize, PAGE_SIZE, Paddr, PageFlags,
        PageProperty, PrivilegedPageFlags, Segment,
        kspace::kvirt_area::KVirtArea,
        page_table::vaddr_range,
        tlb::{TlbFlushOp, TlbFlusher},
    },
    task::disable_preempt,
    util::range_alloc::RangeAllocator,
};
#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
use crate::{sync::SpinLock, util::range_counter::RangeCounter};

/// Metadata for frames behind a [`KVirtArea`] of different page table flags.
///
/// DMA frames that are shared (in a CVM guest) or that are uncachable (for
/// non-DMA-coherent devices) cannot be accessed via linear mappings. Doing so
/// can cause unexpected side effects (e.g., exceptions in a CVM guest or
/// undefined cache behaviors). Therefore, we don't mark these frames as
/// untyped memory, meaning users outside of OSTD can no longer safely create
/// [`VmReader`]s or [`VmWriter`]s on [`Segment<DmaBufferMeta>`].
///
/// [`VmReader`]: crate::mm::VmReader
/// [`VmWriter`]: crate::mm::VmWriter
struct DmaBufferMeta;
impl_frame_meta_for!(DmaBufferMeta);

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

pub(super) fn cvm_need_private_protection() -> bool {
    #[cfg(target_arch = "x86_64")]
    {
        crate::arch::if_tdx_enabled!({ true } else { false })
    }
    #[cfg(not(target_arch = "x86_64"))]
    {
        false
    }
}

pub(super) fn split_daddr(daddr: Option<Daddr>, offset: usize) -> (Option<Daddr>, Option<Daddr>) {
    match daddr {
        Some(daddr) => {
            let daddr1 = daddr;
            let daddr2 = daddr + offset;
            (Some(daddr1), Some(daddr2))
        }
        None => (None, None),
    }
}

pub(super) fn alloc_kva(
    nframes: usize,
    is_cache_coherent: bool,
) -> Result<(KVirtArea, Paddr), Error> {
    let segment = Segment::from_unsized(
        FrameAllocOptions::new().alloc_segment_with(nframes, |_| DmaBufferMeta)?,
    );

    #[cfg_attr(not(target_arch = "x86_64"), expect(unused_labels))]
    let priv_flags = 'priv_flags: {
        #[cfg(target_arch = "x86_64")]
        crate::if_tdx_enabled!({ break 'priv_flags PrivilegedPageFlags::SHARED });

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

    Ok((kva, paddr))
}

/// Prepares a physical address range for DMA mapping.
///
/// If DMA remapping is enabled, the function allocates device addresses, maps
/// the given physical address range to them, and returns the start device
/// address.
///
/// # Safety
///
/// The provided physical address range must be untyped DMA memory that
/// outlives the following [`unprepare_dma()`] call.
pub(super) unsafe fn prepare_dma(pa_range: &Range<Paddr>) -> Option<Daddr> {
    // SAFETY: The safety condition is upheld by the caller.
    #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
    unsafe {
        alloc_unprotect_physical_range(pa_range);
    }

    // SAFETY: The safety condition is upheld by the caller.
    unsafe { dma_remap(pa_range) }
}

/// Unprepares a physical address range from DMA mapping.
///
/// # Safety
///
/// The provided physical address range must be untyped DMA memory that was
/// previously prepared by an [`alloc_unprotect_physical_range()`] call.
pub(super) unsafe fn unprepare_dma(pa_range: &Range<Paddr>, daddr: Option<Daddr>) {
    unmap_dma_remap(daddr.map(|daddr| daddr..daddr + pa_range.len()));

    // SAFETY: The safety condition is upheld by the caller.
    #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
    unsafe {
        dealloc_protect_physical_range(pa_range);
    }
}

/// Marks a physical address range as used (also unprotected if in TDX guest).
///
/// # Safety
///
/// The provided physical address range must be untyped DMA memory that
/// outlives the following [`dealloc_protect_physical_range()`] call.
#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
unsafe fn alloc_unprotect_physical_range(pa_range: &Range<Paddr>) {
    use alloc::{vec, vec::Vec};

    let mut refcnts = PADDR_REF_CNTS.lock();
    let ranges = refcnts.add(pa_range);
    #[cfg(target_arch = "x86_64")]
    crate::arch::if_tdx_enabled!({
        for partial in ranges {
            debug_assert_eq!(partial, pa_range.clone());
            // SAFETY:
            //  - The provided physical address is page aligned.
            //  - The provided physical address range is in bounds.
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

/// Unmarks a physical address range as used (also protected if in TDX guest).
///
/// # Safety
///
/// The provided physical address range must be untyped DMA memory that was
/// previously marked by an [`alloc_unprotect_physical_range()`] call.
#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
unsafe fn dealloc_protect_physical_range(pa_range: &Range<Paddr>) {
    let mut refcnts = PADDR_REF_CNTS.lock();
    let _removed_frames = refcnts.remove(pa_range);
    #[cfg(target_arch = "x86_64")]
    crate::arch::if_tdx_enabled!({
        for pa_range in _removed_frames {
            // SAFETY:
            //  - The provided physical address is page aligned.
            //  - The provided physical address range is in bounds.
            //  - All of the physical pages are untyped memory.
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

/// Allocates device addresses and maps them to the given physical address range.
///
/// # Safety
///
/// The provided physical address range must be untyped DMA memory that
/// outlives the following [`unmap_dma_remap()`] call.
unsafe fn dma_remap(pa_range: &Range<Paddr>) -> Option<Daddr> {
    if has_dma_remapping() {
        #[cfg(target_arch = "x86_64")]
        let daddr = DADDR_ALLOCATOR
            .alloc(pa_range.len())
            .expect("Failed to allocate DMA address range");
        #[cfg(not(target_arch = "x86_64"))]
        let daddr = pa_range.clone();

        for map_paddr in pa_range.clone().step_by(PAGE_SIZE) {
            let map_daddr = (map_paddr - pa_range.start + daddr.start) as Daddr;
            // SAFETY: The caller guarantees that `map_paddr` corresponds to
            // untyped frames that outlive `iommu::unmap()` in `dma_unmap()`.
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
            // FIXME: Flush IOTLBs to prevent any future DMA access to the frames.
        }
    }
}
