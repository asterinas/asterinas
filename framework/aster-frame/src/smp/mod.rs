// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicBool, Ordering};

use log::debug;
use spin::Once;

use crate::{
    arch::{
        enable_common_cpu_features,
        smp::{get_processor_info, init_boot_stack_array, send_boot_ipis},
    },
    cpu,
    sync::SpinLock,
    trap,
    vm::{paddr_to_vaddr, VmAllocOptions, VmIo, VmSegment, PAGE_SIZE},
};

static AP_BOOT_INFO: Once<SpinLock<ApBootInfo>> = Once::new();

const AP_BOOT_STACK_SIZE: usize = PAGE_SIZE * 64;

struct ApBootInfo {
    /// It holds the boot stack top pointers used by all APs.
    boot_stack_array: VmSegment,
    /// `per_ap_info` maps each AP's ID to its associated boot information.
    per_ap_info: BTreeMap<u32, PerApInfo>,
}

struct PerApInfo {
    is_started: AtomicBool,
    // TODO: When the AP starts up and begins executing tasks, the boot stack will
    // no longer be used, and the `VmSegment` can be deallocated (this problem also
    // exists in the boot processor, but the memory it occupies should be returned
    // to the frame allocator).
    boot_stack_frames: VmSegment,
}

pub static CPUNUM: Once<u32> = Once::new();

static AP_LATE_ENTRY: Once<fn() -> !> = Once::new();

/// Only initialize the processor number here to facilitate the system
/// to pre-allocate some data structures according to this number.
pub fn init() {
    let processor_info = get_processor_info();
    let num_processors = match processor_info {
        Some(info) => info.application_processors.len() + 1,
        None => 1,
    };
    CPUNUM.call_once(|| num_processors as u32);
}

/// Boot all application processors.
///
/// This function should be called late in the system startup.
/// The system must at least ensure that the scheduler, ACPI table, memory allocation,
/// and IPI module have been initialized.
pub fn boot_all_aps() {
    // TODO: Adapt to support boot methods without ACPI tables, e.g., Multiboot
    let Some(processor_info) = get_processor_info() else {
        return;
    };
    AP_BOOT_INFO.call_once(|| {
        let mut per_ap_info = BTreeMap::new();
        // Use two pages to place stack pointers of all aps, thus support up to 1024 aps.
        // stack_pointer = *(stack_pointer_array + local_apic_id*8)
        let boot_stack_array = VmAllocOptions::new(2)
            .is_contiguous(true)
            .uninit(false)
            .alloc_contiguous()
            .unwrap();
        for ap in &processor_info.application_processors {
            debug!("application processor info : {:?}", ap);
            let boot_stack_frames = VmAllocOptions::new(AP_BOOT_STACK_SIZE / PAGE_SIZE)
                .is_contiguous(true)
                .uninit(false)
                .alloc_contiguous()
                .unwrap();
            boot_stack_array
                .write_val(
                    8 * ap.local_apic_id as usize,
                    &(paddr_to_vaddr(boot_stack_frames.end_paddr())),
                )
                .unwrap();
            debug!(
                "{} ap_boot_stack_top value: 0x{:X}",
                ap.local_apic_id,
                paddr_to_vaddr(boot_stack_frames.end_paddr())
            );
            per_ap_info.insert(
                ap.local_apic_id,
                PerApInfo {
                    is_started: AtomicBool::new(false),
                    boot_stack_frames,
                },
            );
        }
        init_boot_stack_array(&boot_stack_array);
        SpinLock::new(ApBootInfo {
            boot_stack_array,
            per_ap_info,
        })
    });
    send_boot_ipis();
}

pub fn register_ap_late_entry(entry: fn() -> !) {
    AP_LATE_ENTRY.call_once(|| entry);
}

#[no_mangle]
fn ap_early_entry(local_apic_id: u32) -> ! {
    enable_common_cpu_features();
    cpu::ap_init(local_apic_id);
    trap::init();

    let ap_boot_info = AP_BOOT_INFO.get().unwrap().lock_irq_disabled();
    ap_boot_info
        .per_ap_info
        .get(&local_apic_id)
        .unwrap()
        .is_started
        .store(true, Ordering::Release);
    drop(ap_boot_info);

    let ap_late_entry = AP_LATE_ENTRY.wait();
    ap_late_entry();
}
