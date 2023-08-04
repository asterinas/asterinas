//! Information of memory regions in the boot phase.
//!

use alloc::{vec, vec::Vec};

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
    pub base: usize,
    pub len: usize,
    pub typ: MemoryRegionType,
}

impl MemoryRegion {
    /// Construct a page aligned memory region.
    pub fn new(base: usize, len: usize, typ: MemoryRegionType) -> Self {
        MemoryRegion {
            base: base,
            len: len,
            typ: typ,
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

    /// The type of the region.
    pub fn typ(&self) -> MemoryRegionType {
        self.typ
    }

    /// Remove range t from self, resulting in 0, 1 or 2 truncated ranges.
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
        } else {
            if self.base < t.base + t.len {
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
}
