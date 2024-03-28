// SPDX-License-Identifier: MPL-2.0

use alloc::collections::BTreeMap;
use core::sync::atomic::{AtomicBool, Ordering};

use log::debug;
use spin::Once;

use crate::{
    arch::{
        self, enable_common_cpu_features,
        smp::{get_processor_info, init_boot_stack_array, prepare_boot_stacks, send_boot_ipis},
    },
    cpu,
    sync::SpinLock,
    trap,
    vm::{paddr_to_vaddr, VmAllocOptions, VmIo, VmSegment},
};

static AP_BOOT_INFO: Once<SpinLock<ApBootInfo>> = Once::new();

struct ApBootInfo {
    /// It holds the boot stack top pointers used by all APs.
    boot_stark_array: VmSegment,
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
        let boot_stark_array = VmAllocOptions::new(2)
            .is_contiguous(true)
            .uninit(false)
            .alloc_contiguous()
            .unwrap();
        for ap in &processor_info.application_processors {
            debug!("application processor info : {:?}", ap);
            let boot_stack_frames = prepare_boot_stacks(ap);
            boot_stark_array
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
        init_boot_stack_array(&boot_stark_array);
        SpinLock::new(ApBootInfo {
            boot_stark_array,
            per_ap_info,
        })
    });
    send_boot_ipis();
}

#[no_mangle]
fn ap_early_entry(local_apic_id: u32) -> ! {
    enable_common_cpu_features();
    cpu::ap_init(local_apic_id);
    trap::init();
    arch::init_ap();

    let ap_boot_info = AP_BOOT_INFO.get().unwrap().lock_irq_disabled();
    ap_boot_info
        .per_ap_info
        .get(&local_apic_id)
        .unwrap()
        .is_started
        .store(true, Ordering::Release);
    drop(ap_boot_info);

    #[cfg(not(ktest))]
    unsafe {
        // The entry point of kernel code, which should be defined using the `aster_ap_entry`
        // macro for packaging.
        extern "Rust" {
            fn __aster_ap_entry() -> !;
        }
        __aster_ap_entry();
    }
    #[cfg(ktest)]
    // TODO: Implement multi-processor support to enable parallel testing with ktest.
    loop {}
}
