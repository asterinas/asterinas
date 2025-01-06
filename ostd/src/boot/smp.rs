// SPDX-License-Identifier: MPL-2.0

//! Symmetric multiprocessing (SMP) boot support.

use alloc::{boxed::Box, vec::Vec};
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Once;

use crate::{
    arch::boot::smp::bringup_all_aps,
    cpu,
    mm::{
        frame::{meta::KernelMeta, Segment},
        paddr_to_vaddr, FrameAllocOptions, PAGE_SIZE,
    },
    task::Task,
};

static AP_BOOT_INFO: Once<ApBootInfo> = Once::new();

const AP_BOOT_STACK_SIZE: usize = PAGE_SIZE * 64;

struct ApBootInfo {
    /// Raw boot information for each AP.
    per_ap_raw_info: Segment<KernelMeta>,
    /// Boot information for each AP.
    per_ap_info: Box<[PerApInfo]>,
}

struct PerApInfo {
    is_started: AtomicBool,
    // TODO: When the AP starts up and begins executing tasks, the boot stack will
    // no longer be used, and the `Segment` can be deallocated (this problem also
    // exists in the boot processor, but the memory it occupies should be returned
    // to the frame allocator).
    #[expect(dead_code)]
    boot_stack_pages: Segment<KernelMeta>,
}

/// Raw boot information for APs.
///
/// This is "raw" information that the assembly code (run by APs at startup,
/// before ever entering the Rust entry point) will directly access. So the
/// layout is important. **Update the assembly code if the layout is changed!**
#[repr(C)]
#[derive(Clone, Copy)]
pub(crate) struct PerApRawInfo {
    stack_top: *mut u8,
    cpu_local: *mut u8,
}

static AP_LATE_ENTRY: Once<fn()> = Once::new();

/// Boots all application processors.
///
/// This function should be called late in the system startup. The system must at
/// least ensure that the scheduler, ACPI table, memory allocation, and IPI module
/// have been initialized.
///
/// # Safety
///
/// This function can only be called in the boot context of the BSP where APs have
/// not yet been booted.
pub(crate) unsafe fn boot_all_aps() {
    let num_cpus = crate::cpu::num_cpus();

    if num_cpus == 1 {
        return;
    }
    log::info!("Booting {} processors", num_cpus - 1);

    // We currently assume that
    // 1. the bootstrap processor (BSP) has the processor ID 0;
    // 2. the processor ID starts from `0` to `num_cpus - 1`.

    let mut per_ap_info = Vec::new();

    let per_ap_raw_info = FrameAllocOptions::new()
        .zeroed(false)
        .alloc_segment_with(
            num_cpus
                .saturating_sub(1)
                .checked_mul(core::mem::size_of::<PerApRawInfo>())
                .unwrap()
                .div_ceil(PAGE_SIZE),
            |_| KernelMeta,
        )
        .unwrap();
    let raw_info_ptr = paddr_to_vaddr(per_ap_raw_info.start_paddr()) as *mut PerApRawInfo;

    for ap in 1..num_cpus {
        let boot_stack_pages = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(AP_BOOT_STACK_SIZE / PAGE_SIZE, |_| KernelMeta)
            .unwrap();

        let raw_info = PerApRawInfo {
            stack_top: paddr_to_vaddr(boot_stack_pages.end_paddr()) as *mut u8,
            cpu_local: paddr_to_vaddr(crate::cpu::local::get_ap(ap.try_into().unwrap())) as *mut u8,
        };

        // SAFETY: The index is in range because we allocated enough memory.
        let ptr = unsafe { raw_info_ptr.add(ap - 1) };
        // SAFETY: The memory is valid for writing because it was just allocated.
        unsafe { ptr.write(raw_info) };

        per_ap_info.push(PerApInfo {
            is_started: AtomicBool::new(false),
            boot_stack_pages,
        });
    }

    assert!(!AP_BOOT_INFO.is_completed());
    AP_BOOT_INFO.call_once(move || ApBootInfo {
        per_ap_raw_info,
        per_ap_info: per_ap_info.into_boxed_slice(),
    });

    log::info!("Booting all application processors...");

    let info_ptr = paddr_to_vaddr(AP_BOOT_INFO.get().unwrap().per_ap_raw_info.start_paddr())
        as *mut PerApRawInfo;
    let pt_ptr = crate::mm::page_table::boot_pt::with_borrow(|pt| pt.root_address()).unwrap();
    // SAFETY: It's the right time to boot APs (guaranteed by the caller) and
    // the arguments are valid to boot APs (generated above).
    unsafe { bringup_all_aps(info_ptr, pt_ptr, num_cpus as u32) };

    wait_for_all_aps_started();

    log::info!("All application processors started. The BSP continues to run.");
}

/// Register the entry function for the application processor.
///
/// Once the entry function is registered, all the application processors
/// will jump to the entry function immediately.
pub fn register_ap_entry(entry: fn()) {
    AP_LATE_ENTRY.call_once(|| entry);
}

#[no_mangle]
fn ap_early_entry(cpu_id: u32) -> ! {
    // SAFETY: `cpu_id` is the correct value of the CPU ID.
    unsafe {
        cpu::init_on_ap(cpu_id);
    }

    crate::arch::enable_cpu_features();

    // SAFETY: this function is only called once on this AP.
    unsafe {
        crate::arch::trap::init(false);
    }

    // SAFETY: this function is only called once on this AP, after the BSP has
    // done the architecture-specific initialization.
    unsafe {
        crate::arch::init_on_ap();
    }

    crate::arch::irq::enable_local();

    // SAFETY: this function is only called once on this AP.
    unsafe {
        crate::mm::kspace::activate_kernel_page_table();
    }

    // Mark the AP as started.
    let ap_boot_info = AP_BOOT_INFO.get().unwrap();
    ap_boot_info.per_ap_info[cpu_id as usize - 1]
        .is_started
        .store(true, Ordering::Release);

    log::info!("Processor {} started. Spinning for tasks.", cpu_id);

    let ap_late_entry = AP_LATE_ENTRY.wait();
    ap_late_entry();

    Task::yield_now();
    unreachable!("`yield_now` in the boot context should not return");
}

fn wait_for_all_aps_started() {
    fn is_all_aps_started() -> bool {
        let ap_boot_info = AP_BOOT_INFO.get().unwrap();
        ap_boot_info
            .per_ap_info
            .iter()
            .all(|info| info.is_started.load(Ordering::Acquire))
    }

    while !is_all_aps_started() {
        core::hint::spin_loop();
    }
}
