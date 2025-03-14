// SPDX-License-Identifier: MPL-2.0

//! Virtual memory (VM).
//!
//! There are two primary VM abstractions:
//!  * Virtual Memory Address Regions (VMARs) a type of capability that manages
//!    user address spaces.
//!  * Virtual Memory Objects (VMOs) are are a type of capability that
//!    represents a set of memory pages.
//!
//! The concepts of VMARs and VMOs are originally introduced by
//! [Zircon](https://fuchsia.dev/fuchsia-src/reference/kernel_objects/vm_object).
//! As capabilities, the two abstractions are aligned with our goal
//! of everything-is-a-capability, although their specifications and
//! implementations in C/C++ cannot apply directly to Asterinas.
//! In Asterinas, VMARs and VMOs, as well as other capabilities, are implemented
//! as zero-cost capabilities.

use osdk_frame_allocator::FrameAllocator;
use ostd::{cpu::CpuExceptionInfo, task::Task};

use crate::{prelude::*, thread::exception::handle_page_fault_from_vmar};

pub mod page_fault_handler;
pub mod perms;
pub mod util;
pub mod vmar;
pub mod vmo;

#[ostd::global_frame_allocator]
static FRAME_ALLOCATOR: FrameAllocator = FrameAllocator;

/// Total physical memory in the entire system in bytes.
pub fn mem_total() -> usize {
    use ostd::boot::{boot_info, memory_region::MemoryRegionType};

    let regions = &boot_info().memory_regions;
    let total = regions
        .iter()
        .filter(|region| region.typ() == MemoryRegionType::Usable)
        .map(|region| region.len())
        .sum::<usize>();

    total
}

fn page_fault_handler(info: &CpuExceptionInfo) -> core::result::Result<(), ()> {
    let task = Task::current().unwrap();
    let root_vmar = task.as_thread_local().unwrap().root_vmar().borrow();
    handle_page_fault_from_vmar(root_vmar.as_ref().unwrap(), &info.try_into().unwrap())
}

pub(super) fn init() {
    ostd::arch::trap::inject_user_page_fault_handler(page_fault_handler);
}
