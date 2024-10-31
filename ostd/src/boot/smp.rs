// SPDX-License-Identifier: MPL-2.0

//! Symmetric multiprocessing (SMP) boot support.

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Once;

use crate::{
    arch::boot::smp::{bringup_all_aps, get_num_processors},
    cpu,
    mm::{
        paddr_to_vaddr,
        page::{self, meta::KernelMeta, ContPages},
        PAGE_SIZE,
    },
    task::Task,
};

pub(crate) static AP_BOOT_INFO: Once<ApBootInfo> = Once::new();

const AP_BOOT_STACK_SIZE: usize = PAGE_SIZE * 64;

pub(crate) struct ApBootInfo {
    /// It holds the boot stack top pointers used by all APs.
    pub(crate) boot_stack_array: ContPages<KernelMeta>,
    /// `per_ap_info` maps each AP's ID to its associated boot information.
    per_ap_info: BTreeMap<u32, PerApInfo>,
}

struct PerApInfo {
    is_started: AtomicBool,
    // TODO: When the AP starts up and begins executing tasks, the boot stack will
    // no longer be used, and the `ContPages` can be deallocated (this problem also
    // exists in the boot processor, but the memory it occupies should be returned
    // to the frame allocator).
    boot_stack_pages: ContPages<KernelMeta>,
}

static AP_LATE_ENTRY: Once<fn()> = Once::new();

/// Boot all application processors.
///
/// This function should be called late in the system startup. The system must at
/// least ensure that the scheduler, ACPI table, memory allocation, and IPI module
/// have been initialized.
///
/// However, the function need to be called before any `cpu_local!` variables are
/// accessed, including the APIC instance.
pub fn boot_all_aps() {
    // TODO: support boot protocols without ACPI tables, e.g., Multiboot
    let Some(num_cpus) = get_num_processors() else {
        log::warn!("No processor information found. The kernel operates with a single processor.");
        return;
    };
    log::info!("Found {} processors.", num_cpus);

    // We currently assumes that bootstrap processor (BSP) have always the
    // processor ID 0. And the processor ID starts from 0 to `num_cpus - 1`.

    AP_BOOT_INFO.call_once(|| {
        let mut per_ap_info = BTreeMap::new();
        // Use two pages to place stack pointers of all APs, thus support up to 1024 APs.
        let boot_stack_array =
            page::allocator::alloc_contiguous(2 * PAGE_SIZE, |_| KernelMeta::default()).unwrap();
        assert!(num_cpus < 1024);

        for ap in 1..num_cpus {
            let boot_stack_pages =
                page::allocator::alloc_contiguous(AP_BOOT_STACK_SIZE, |_| KernelMeta::default())
                    .unwrap();
            let boot_stack_ptr = paddr_to_vaddr(boot_stack_pages.end_paddr());
            let stack_array_ptr = paddr_to_vaddr(boot_stack_array.start_paddr()) as *mut u64;
            // SAFETY: The `stack_array_ptr` is valid and aligned.
            unsafe {
                stack_array_ptr
                    .add(ap as usize)
                    .write_volatile(boot_stack_ptr as u64);
            }
            per_ap_info.insert(
                ap,
                PerApInfo {
                    is_started: AtomicBool::new(false),
                    boot_stack_pages,
                },
            );
        }

        ApBootInfo {
            boot_stack_array,
            per_ap_info,
        }
    });

    log::info!("Booting all application processors...");

    bringup_all_aps();
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
fn ap_early_entry(local_apic_id: u32) -> ! {
    crate::arch::enable_cpu_features();

    // SAFETY: we are on the AP and they are only called once with the correct
    // CPU ID.
    unsafe {
        cpu::local::init_on_ap(local_apic_id);
        cpu::set_this_cpu_id(local_apic_id);
    }

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
    ap_boot_info
        .per_ap_info
        .get(&local_apic_id)
        .unwrap()
        .is_started
        .store(true, Ordering::Release);

    log::info!("Processor {} started. Spinning for tasks.", local_apic_id);

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
            .values()
            .all(|info| info.is_started.load(Ordering::Acquire))
    }

    while !is_all_aps_started() {
        core::hint::spin_loop();
    }
}
