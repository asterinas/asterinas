// SPDX-License-Identifier: MPL-2.0

//! User address space management.

mod interval_set;
mod util;
mod vm_mapping;

mod vmar_impls;

use ostd::mm::Vaddr;
pub use vmar_impls::{RssType, Vmar};

pub const VMAR_LOWEST_ADDR: Vaddr = 0x001_0000; // 64 KiB is the Linux configurable default
pub const VMAR_CAP_ADDR: Vaddr = ostd::mm::MAX_USERSPACE_VADDR;

/// Returns whether the input `vaddr` is a legal user space virtual address.
pub fn is_userspace_vaddr(vaddr: Vaddr) -> bool {
    (VMAR_LOWEST_ADDR..VMAR_CAP_ADDR).contains(&vaddr)
}

/// Returns whether `vaddr` and `len` specify a legal user space virtual address range.
fn is_userspace_vaddr_range(vaddr: Vaddr, len: usize) -> bool {
    vaddr >= VMAR_LOWEST_ADDR
        && VMAR_CAP_ADDR
            .checked_sub(vaddr)
            .is_some_and(|gap| gap >= len)
}
