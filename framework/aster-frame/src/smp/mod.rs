use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicBool, Ordering};

use spin::Once;

use crate::{
    arch::{
        enable_common_cpu_features,
        smp::{get_processor_info, init_boot_stack_array, send_boot_ipis},
    },
    cpu,
    sync::SpinLock,
    trap,
    vm::VmSegment,
};

static AP_BOOT_INFO: Once<SpinLock<BTreeMap<u32, ApBootInfo>>> = Once::new();

struct ApBootInfo {
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
    let num_aps = processor_info.application_processors.len();
    CPUNUM.call_once(|| (num_aps + 1) as u32);
}

/// Boot all application processors.
///
/// This function should be called late in the system startup.
/// The system must at least ensure that the scheduler, ACPI table, memory allocation,
/// and IPI module have been initialized.
pub fn boot_all_aps() {
    let processor_info = get_processor_info();
    AP_BOOT_INFO.call_once(|| {
        let mut ap_boot_info = BTreeMap::new();
        for ap in &processor_info.application_processors {
            let boot_stack_frames = prepare_boot_stacks(ap);
            ap_boot_info.insert(
                ap.local_apic_id,
                ApBootInfo {
                    is_started: AtomicBool::new(false),
                    boot_stack_frames,
                },
            );
        }
        SpinLock::new(ap_boot_info)
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
        .get(&local_apic_id)
        .unwrap()
        .is_started
        .store(true, Ordering::Release);
    drop(ap_boot_info);

    let ap_late_entry = AP_LATE_ENTRY.wait();
    ap_late_entry();
}
