// SPDX-License-Identifier: MPL-2.0

use core::{fmt::Debug, mem::ManuallyDrop};

use super::util::{
    alloc_kva, cvm_need_private_protection, prepare_dma, split_daddr, unprepare_dma,
};
use crate::{
    error::Error,
    mm::{
        Daddr, FrameAllocOptions, HasDaddr, HasPaddr, HasPaddrRange, HasSize, Infallible,
        PAGE_SIZE, Paddr, Segment, Split, VmReader, VmWriter,
        io_util::{HasVmReaderWriter, VmReaderWriterIdentity},
        kspace::kvirt_area::KVirtArea,
    },
};

/// A DMA memory object that can be accessed in a cache-coherent manner.
///
/// The users need not manually synchronize the CPU cache and the device when
/// accessing the memory region with [`VmReader`] and [`VmWriter`]. If the
/// device doesn't not support cache-coherent access, the memory region will be
/// mapped without caching enabled.
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
    /// The `is_cache_coherent` argument specifies whether the target device
    /// that the DMA mapping is prepared for can access the main memory in a
    /// CPU cache coherent way or not.
    pub fn alloc(nframes: usize, is_cache_coherent: bool) -> Result<Self, Error> {
        let cvm = cvm_need_private_protection();

        let (inner, paddr_range) = if is_cache_coherent && !cvm {
            let segment = FrameAllocOptions::new().alloc_segment(nframes)?;
            let paddr_range = segment.paddr_range();

            (Inner::Segment(segment), paddr_range)
        } else {
            let (kva, paddr) = alloc_kva(nframes, is_cache_coherent)?;

            (Inner::Kva(kva, paddr), paddr..paddr + nframes * PAGE_SIZE)
        };

        // SAFETY: The physical address range is untyped DMA memory before `drop`.
        let map_daddr = unsafe { prepare_dma(&paddr_range) };

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
                let (paddr1, paddr2) = (paddr, paddr + offset);
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
        // SAFETY: The physical address range was prepeared in `alloc`.
        unsafe { unprepare_dma(&self.paddr_range(), self.map_daddr) };
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

impl HasDaddr for DmaCoherent {
    fn daddr(&self) -> Daddr {
        self.map_daddr.unwrap_or_else(|| self.paddr() as Daddr)
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
