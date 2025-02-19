// SPDX-License-Identifier: MPL-2.0

//! Information of memory regions in the boot phase.
//!

use core::ops::Deref;

use crate::mm::{
    kspace::{kernel_loaded_offset, KERNEL_CODE_BASE_VADDR, LINEAR_MAPPING_BASE_VADDR},
    Paddr,
};

/// The type of initial memory regions that are needed for the kernel.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub enum MemoryRegionType {
    /// Maybe points to an unplugged DIMM module. It's bad anyway.
    BadMemory = 0,
    /// In ACPI spec, this area needs to be preserved when sleeping.
    NonVolatileSleep = 1,
    /// Reserved by BIOS or bootloader, do not use.
    Reserved = 2,
    /// The place where kernel sections are loaded.
    Kernel = 3,
    /// The place where kernel modules (e.g. initrd) are loaded, could be reused.
    Module = 4,
    /// The memory region provided as the framebuffer.
    Framebuffer = 5,
    /// Once used in the boot phase. Kernel can reclaim it after initialization.
    Reclaimable = 6,
    /// Directly usable by the frame allocator.
    Usable = 7,
}

/// The information of initial memory regions that are needed by the kernel.
/// The sections are **not** guaranteed to not overlap. The region must be page aligned.
#[derive(Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Debug)]
pub struct MemoryRegion {
    base: usize,
    len: usize,
    typ: MemoryRegionType,
}

impl MemoryRegion {
    /// Constructs a valid memory region.
    pub const fn new(base: usize, len: usize, typ: MemoryRegionType) -> Self {
        MemoryRegion { base, len, typ }
    }

    /// Constructs a bad memory region.
    pub const fn bad() -> Self {
        MemoryRegion {
            base: 0,
            len: 0,
            typ: MemoryRegionType::BadMemory,
        }
    }

    /// Constructs a memory region where kernel sections are loaded.
    ///
    /// Most boot protocols do not mark the place where the kernel loads as unusable. In this case,
    /// we need to explicitly construct and append this memory region.
    pub fn kernel() -> Self {
        // These are physical addresses provided by the linker script.
        extern "C" {
            fn __kernel_start();
            fn __kernel_end();
        }
        MemoryRegion {
            base: __kernel_start as usize - kernel_loaded_offset(),
            len: __kernel_end as usize - __kernel_start as usize,
            typ: MemoryRegionType::Kernel,
        }
    }

    /// Constructs a memory region from a slice of early boot data.
    ///
    /// This helps marking the memory containing early boot data, as it may not
    /// be sent to the frame allocator but it is reclaimable after boot.
    pub fn from_early_str(slice: &str) -> Self {
        let mut base = slice.as_ptr() as Paddr;

        if base > KERNEL_CODE_BASE_VADDR {
            base -= KERNEL_CODE_BASE_VADDR;
        } else if base > LINEAR_MAPPING_BASE_VADDR {
            base -= LINEAR_MAPPING_BASE_VADDR;
        }

        MemoryRegion {
            base,
            len: slice.len(),
            typ: MemoryRegionType::Reclaimable,
        }
    }

    /// The physical address of the base of the region.
    pub fn base(&self) -> usize {
        self.base
    }

    /// The length in bytes of the region.
    pub fn len(&self) -> usize {
        self.len
    }

    /// The physical address of the end of the region.
    pub fn end(&self) -> usize {
        self.base + self.len
    }

    /// Checks whether the region is empty
    pub fn is_empty(&self) -> bool {
        self.len == 0
    }

    /// The type of the region.
    pub fn typ(&self) -> MemoryRegionType {
        self.typ
    }

    /// Removes range `t` from self, resulting in 0, 1 or 2 truncated ranges.
    /// We need to have this method since memory regions can overlap.
    pub fn truncate(&self, t: &MemoryRegion) -> MemoryRegionArray<2> {
        if self.base < t.base {
            if self.base + self.len > t.base {
                if self.base + self.len > t.base + t.len {
                    MemoryRegionArray::from(&[
                        MemoryRegion {
                            base: self.base,
                            len: t.base - self.base,
                            typ: self.typ,
                        },
                        MemoryRegion {
                            base: t.base + t.len,
                            len: self.base + self.len - (t.base + t.len),
                            typ: self.typ,
                        },
                    ])
                } else {
                    MemoryRegionArray::from(&[MemoryRegion {
                        base: self.base,
                        len: t.base - self.base,
                        typ: self.typ,
                    }])
                }
            } else {
                MemoryRegionArray::from(&[*self])
            }
        } else if self.base < t.base + t.len {
            if self.base + self.len > t.base + t.len {
                MemoryRegionArray::from(&[MemoryRegion {
                    base: t.base + t.len,
                    len: self.base + self.len - (t.base + t.len),
                    typ: self.typ,
                }])
            } else {
                MemoryRegionArray::new()
            }
        } else {
            MemoryRegionArray::from(&[*self])
        }
    }
}

/// The maximum number of regions that can be handled.
///
/// The choice of 512 is probably fine since old Linux boot protocol only
/// allows 128 regions.
//
// TODO: confirm the number or make it configurable.
pub const MAX_REGIONS: usize = 512;

/// A heapless set of memory regions.
///
/// The set cannot contain more than `LEN` regions.
pub struct MemoryRegionArray<const LEN: usize = MAX_REGIONS> {
    regions: [MemoryRegion; LEN],
    count: usize,
}

impl<const LEN: usize> Default for MemoryRegionArray<LEN> {
    fn default() -> Self {
        Self::new()
    }
}

impl<const LEN: usize> Deref for MemoryRegionArray<LEN> {
    type Target = [MemoryRegion];

    fn deref(&self) -> &Self::Target {
        &self.regions[..self.count]
    }
}

impl<const LEN: usize> MemoryRegionArray<LEN> {
    /// Constructs an empty set.
    pub const fn new() -> Self {
        Self {
            regions: [MemoryRegion::bad(); LEN],
            count: 0,
        }
    }

    /// Constructs from an array of regions.
    pub fn from(array: &[MemoryRegion]) -> Self {
        Self {
            regions: core::array::from_fn(|i| {
                if i < array.len() {
                    array[i]
                } else {
                    MemoryRegion::bad()
                }
            }),
            count: array.len(),
        }
    }

    /// Appends a region to the set.
    ///
    /// If the set is full, an error is returned.
    pub fn push(&mut self, region: MemoryRegion) -> Result<(), &'static str> {
        if self.count < self.regions.len() {
            self.regions[self.count] = region;
            self.count += 1;
            Ok(())
        } else {
            Err("MemoryRegionArray is full")
        }
    }

    /// Clears the set.
    pub fn clear(&mut self) {
        self.count = 0;
    }

    /// Truncates regions, resulting in a set of regions that does not overlap.
    ///
    /// The truncation will be done according to the type of the regions, that
    /// usable and reclaimable regions will be truncated by the unusable regions.
    ///
    /// If the output regions are more than `LEN`, the extra regions will be ignored.
    pub fn into_non_overlapping(self) -> Self {
        // We should later use regions in `regions_unusable` to truncate all
        // regions in `regions_usable`.
        // The difference is that regions in `regions_usable` could be used by
        // the frame allocator.
        let mut regions_usable = MemoryRegionArray::<LEN>::new();
        let mut regions_unusable = MemoryRegionArray::<LEN>::new();

        for r in self.iter() {
            match r.typ {
                MemoryRegionType::Usable => {
                    // If usable memory regions exceeded it's fine to ignore the rest.
                    let _ = regions_usable.push(*r);
                }
                _ => {
                    regions_unusable
                        .push(*r)
                        .expect("Too many unusable memory regions");
                }
            }
        }

        // `regions_*` are 2 rolling vectors since we are going to truncate
        // the regions in a iterative manner.
        let mut regions = MemoryRegionArray::<LEN>::new();
        let regions_src = &mut regions_usable;
        let regions_dst = &mut regions;
        // Truncate the usable regions.
        for r_unusable in regions_unusable.iter() {
            regions_dst.clear();
            for r_usable in regions_src.iter() {
                for truncated in r_usable.truncate(r_unusable).iter() {
                    let _ = regions_dst.push(*truncated);
                }
            }
            core::mem::swap(regions_src, regions_dst);
        }

        // Combine all the regions processed.
        let mut all_regions = regions_unusable;
        for r in regions_usable.iter() {
            let _ = all_regions.push(*r);
        }
        all_regions
    }
}
