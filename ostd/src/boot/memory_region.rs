// SPDX-License-Identifier: MPL-2.0

//! Information of memory regions in the boot phase.
//!

use alloc::{vec, vec::Vec};
use core::mem::swap;

use crate::mm::kspace::kernel_loaded_offset;

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
    pub fn new(base: usize, len: usize, typ: MemoryRegionType) -> Self {
        MemoryRegion { base, len, typ }
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

    /// The physical address of the base of the region.
    pub fn base(&self) -> usize {
        self.base
    }

    /// The length in bytes of the region.
    pub fn len(&self) -> usize {
        self.len
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
    pub fn truncate(&self, t: &MemoryRegion) -> Vec<MemoryRegion> {
        if self.base < t.base {
            if self.base + self.len > t.base {
                if self.base + self.len > t.base + t.len {
                    vec![
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
                    ]
                } else {
                    vec![MemoryRegion {
                        base: self.base,
                        len: t.base - self.base,
                        typ: self.typ,
                    }]
                }
            } else {
                vec![*self]
            }
        } else if self.base < t.base + t.len {
            if self.base + self.len > t.base + t.len {
                vec![MemoryRegion {
                    base: t.base + t.len,
                    len: self.base + self.len - (t.base + t.len),
                    typ: self.typ,
                }]
            } else {
                vec![]
            }
        } else {
            vec![*self]
        }
    }
}

/// Truncates regions, resulting in a set of regions that does not overlap.
///
/// The truncation will be done according to the type of the regions, that
/// usable and reclaimable regions will be truncated by the unusable regions.
pub fn non_overlapping_regions_from(regions: &[MemoryRegion]) -> Vec<MemoryRegion> {
    // We should later use regions in `regions_unusable` to truncate all
    // regions in `regions_usable`.
    // The difference is that regions in `regions_usable` could be used by
    // the frame allocator.
    let mut regions_usable = Vec::<MemoryRegion>::new();
    let mut regions_unusable = Vec::<MemoryRegion>::new();

    for r in regions {
        match r.typ {
            MemoryRegionType::Usable | MemoryRegionType::Reclaimable => {
                regions_usable.push(*r);
            }
            _ => {
                regions_unusable.push(*r);
            }
        }
    }

    // `regions_*` are 2 rolling vectors since we are going to truncate
    // the regions in a iterative manner.
    let mut regions = Vec::<MemoryRegion>::new();
    let regions_src = &mut regions_usable;
    let regions_dst = &mut regions;
    // Truncate the usable regions.
    for &r_unusable in &regions_unusable {
        regions_dst.clear();
        for r_usable in &*regions_src {
            regions_dst.append(&mut r_usable.truncate(&r_unusable));
        }
        swap(regions_src, regions_dst);
    }

    // Combine all the regions processed.
    let mut all_regions = regions_unusable;
    all_regions.append(&mut regions_usable);
    all_regions
}
