// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Debug, mem::ManuallyDrop};

use super::util::{alloc_kva, dma_remap, has_tdx, split_daddr, unmap_dma_remap};
#[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
use super::util::{alloc_unprotect_physical_range, dealloc_protect_physical_range};
use crate::{
    error::Error,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasPaddr, HasPaddrRange, HasSize, Infallible,
        PAGE_SIZE, Paddr, Segment, Split, VmReader, VmWriter,
        io_util::{HasVmReaderWriter, VmReaderWriterIdentity},
        kspace::kvirt_area::KVirtArea,
    },
};

/// A DMA memory object with coherent cache.
#[derive(Debug)]
pub struct DmaCoherent {
    inner: Inner,
    map_daddr: Option<Daddr>,
    is_cache_coherent: bool,
}

#[derive(Debug)]
enum Inner {
    Segment(Segment<()>),
    Kva(KVirtArea, Paddr),
}

impl DmaCoherent {
    /// Allocates a region of physical memory for coherent DMA access.
    ///
    /// If the device can access the memory with coherent access to the CPU
    /// cache, set `is_cache_coherent` to `true`.
    pub fn alloc(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        let has_tdx = has_tdx();

        let (inner, paddr_range) = if is_cache_coherent && !has_tdx {
            let segment = FrameAllocOptions::new().alloc_segment(nframes)?;
            let paddr_range = segment.paddr_range();

            (Inner::Segment(segment), paddr_range)
        } else {
            let (kva, paddr) = alloc_kva(nframes * PAGE_SIZE, has_tdx, is_cache_coherent)?;

            (Inner::Kva(kva, paddr), paddr..paddr + nframes * PAGE_SIZE)
        };

        // SAFETY: The physical address range is untyped DMA memory before `drop`.
        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        unsafe {
            alloc_unprotect_physical_range(&paddr_range)
        };

        // SAFETY: The physical address range is untyped DMA memory before `drop`.
        let map_daddr = unsafe { dma_remap(&paddr_range) };

        Ok(Self {
            inner,
            map_daddr,
            is_cache_coherent,
        })
    }
}

impl Split for DmaCoherent {
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
                let paddr1 = paddr;
                let paddr2 = paddr + offset;
                (Inner::Kva(kva1, paddr1), Inner::Kva(kva2, paddr2))
            }
        };

        let (daddr1, daddr2) = split_daddr(map_daddr, offset);

        (
            Self {
                inner: inner1,
                map_daddr: daddr1,
                is_cache_coherent,
            },
            Self {
                inner: inner2,
                map_daddr: daddr2,
                is_cache_coherent,
            },
        )
    }
}

impl Drop for DmaCoherent {
    fn drop(&mut self) {
        // SAFETY: The physical address range was marked in `alloc`.
        #[cfg(any(debug_assertions, all(target_arch = "x86_64", feature = "cvm_guest")))]
        unsafe {
            dealloc_protect_physical_range(&self.paddr_range())
        };

        unmap_dma_remap(self.map_daddr.map(|daddr| daddr..daddr + self.size()));
    }
}

impl HasDaddr for DmaCoherent {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
    }
}

impl HasPaddr for DmaCoherent {
    fn paddr(&self) -> Paddr {
        match &self.inner {
            Inner::Segment(segment) => segment.paddr(),
            Inner::Kva(_, paddr) => *paddr,
        }
    }
}

impl HasSize for DmaCoherent {
    fn size(&self) -> usize {
        match &self.inner {
            Inner::Segment(segment) => segment.size(),
            Inner::Kva(kva, _) => kva.size(),
        }
    }
}

impl HasVmReaderWriter for DmaCoherent {
    type Types = VmReaderWriterIdentity;

    fn reader(&self) -> VmReader<'_, Infallible> {
        match &self.inner {
            Inner::Segment(seg) => seg.reader(),
            Inner::Kva(kva, _) => {
                // SAFETY:
                //  - The memory range points to untyped memory.
                //  - The KVA is alive during the lifetime `'_`.
                //  - Using `VmReader` and `VmWriter` is the only way to access the KVA.
                unsafe { VmReader::from_kernel_space(kva.start() as *const u8, kva.size()) }
            }
        }
    }

    fn writer(&self) -> VmWriter<'_, Infallible> {
        match &self.inner {
            Inner::Segment(seg) => seg.writer(),
            Inner::Kva(kva, _) => {
                // SAFETY:
                //  - The memory range points to untyped memory.
                //  - The KVA is alive during the lifetime `'_`.
                //  - Using `VmReader` and `VmWriter` is the only way to access the KVA.
                unsafe { VmWriter::from_kernel_space(kva.start() as *mut u8, kva.size()) }
            }
        }
    }
}
