// SPDX-License-Identifier: MPL-2.0

//! User address space management.

mod interval_set;
mod util;
mod vm_mapping;

mod vmar_impls;

use core::ops::Range;

use ostd::mm::Vaddr;
pub use vmar_impls::{RssType, Vmar};

pub const VMAR_LOWEST_ADDR: Vaddr = 0x001_0000; // 64 KiB is the Linux configurable default
const VMAR_CAP_ADDR: Vaddr = ostd::mm::MAX_USERSPACE_VADDR;

/// Returns whether the input `vaddr` is a legal user space virtual address.
pub fn is_userspace_vaddr(vaddr: Vaddr) -> bool {
    (VMAR_LOWEST_ADDR..VMAR_CAP_ADDR).contains(&vaddr)
}

/// Returns the full user space virtual address range.
pub fn userspace_range() -> Range<Vaddr> {
    VMAR_LOWEST_ADDR..VMAR_CAP_ADDR
}
