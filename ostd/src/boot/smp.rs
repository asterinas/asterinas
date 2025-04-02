// SPDX-License-Identifier: MPL-2.0

//! Symmetric multiprocessing (SMP) boot support.

use alloc::{boxed::Box, collections::btree_map::BTreeMap, vec::Vec};

use spin::Once;

use crate::{
    arch::{boot::smp::bringup_all_aps, irq::HwCpuId},
    mm::{
        frame::{meta::KernelMeta, Segment},
        paddr_to_vaddr, FrameAllocOptions, PAGE_SIZE,
    },
    sync::SpinLock,
    task::Task,
};

static AP_BOOT_INFO: Once<ApBootInfo> = Once::new();

const AP_BOOT_STACK_SIZE: usize = PAGE_SIZE * 64;

struct ApBootInfo {
    /// Raw boot information for each AP.
    per_ap_raw_info: Box<[PerApRawInfo]>,
    /// Boot information for each AP.
    #[expect(dead_code)]
    per_ap_info: Box<[PerApInfo]>,
}

struct PerApInfo {
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

// SAFETY: This information (i.e., the pointer addresses) can be shared safely
// among multiple threads. However, it is the responsibility of the user to
// ensure that the contained pointers are used safely.
unsafe impl Send for PerApRawInfo {}
unsafe impl Sync for PerApRawInfo {}

static HW_CPU_ID_MAP: SpinLock<BTreeMap<u32, HwCpuId>> = SpinLock::new(BTreeMap::new());

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
    // Mark the BSP as started.
    report_online_and_hw_cpu_id(crate::cpu::CpuId::bsp().as_usize().try_into().unwrap());

    let num_cpus = crate::cpu::num_cpus();

    if num_cpus == 1 {
        return;
    }
    log::info!("Booting {} processors", num_cpus - 1);

    let mut per_ap_raw_info = Vec::with_capacity(num_cpus);
    let mut per_ap_info = Vec::with_capacity(num_cpus);

    for ap in 1..num_cpus {
        let boot_stack_pages = FrameAllocOptions::new()
            .zeroed(false)
            .alloc_segment_with(AP_BOOT_STACK_SIZE / PAGE_SIZE, |_| KernelMeta)
            .unwrap();

        per_ap_raw_info.push(PerApRawInfo {
            stack_top: paddr_to_vaddr(boot_stack_pages.end_paddr()) as *mut u8,
            cpu_local: paddr_to_vaddr(crate::cpu::local::get_ap(ap.try_into().unwrap())) as *mut u8,
        });
        per_ap_info.push(PerApInfo { boot_stack_pages });
    }

    assert!(!AP_BOOT_INFO.is_completed());
    AP_BOOT_INFO.call_once(move || ApBootInfo {
        per_ap_raw_info: per_ap_raw_info.into_boxed_slice(),
        per_ap_info: per_ap_info.into_boxed_slice(),
    });

    log::info!("Booting all application processors...");

    let info_ptr = AP_BOOT_INFO.get().unwrap().per_ap_raw_info.as_ptr();
    let pt_ptr = crate::mm::page_table::boot_pt::with_borrow(|pt| pt.root_address()).unwrap();
    // SAFETY: It's the right time to boot APs (guaranteed by the caller) and
    // the arguments are valid to boot APs (generated above).
    unsafe { bringup_all_aps(info_ptr, pt_ptr, num_cpus as u32) };

    wait_for_all_aps_started(num_cpus);

    log::info!("All application processors started. The BSP continues to run.");
}

static AP_LATE_ENTRY: Once<fn()> = Once::new();

/// Registers the entry function for the application processor.
///
/// Once the entry function is registered, all the application processors
/// will jump to the entry function immediately.
pub fn register_ap_entry(entry: fn()) {
    AP_LATE_ENTRY.call_once(|| entry);
}

#[no_mangle]
fn ap_early_entry(cpu_id: u32) -> ! {
    // SAFETY: `cpu_id` is the correct value of the CPU ID.
    unsafe { crate::cpu::init_on_ap(cpu_id) };

    crate::arch::enable_cpu_features();

    // SAFETY: This function is called in the boot context of the AP.
    unsafe { crate::arch::trap::init() };

    // SAFETY: This function is only called once on this AP, after the BSP has
    // done the architecture-specific initialization.
    unsafe { crate::arch::init_on_ap() };

    #[cfg(feature = "lazy_tlb_flush_on_unmap")]
    crate::mm::tlb::latr::init_this_cpu();

    crate::arch::irq::enable_local();

    // SAFETY: This function is only called once on this AP.
    unsafe { crate::mm::kspace::activate_kernel_page_table() };

    // Mark the AP as started.
    report_online_and_hw_cpu_id(cpu_id);

    log::info!("Processor {} started. Spinning for tasks.", cpu_id);

    let ap_late_entry = AP_LATE_ENTRY.wait();
    ap_late_entry();

    Task::yield_now();
    unreachable!("`yield_now` in the boot context should not return");
}

fn report_online_and_hw_cpu_id(cpu_id: u32) {
    // There are no races because this method will only be called in the boot
    // context, where preemption won't occur.
    let hw_cpu_id = HwCpuId::read_current(&crate::task::disable_preempt());

    let old_val = HW_CPU_ID_MAP.lock().insert(cpu_id, hw_cpu_id);
    assert!(old_val.is_none());
}

fn wait_for_all_aps_started(num_cpus: usize) {
    fn is_all_aps_started(num_cpus: usize) -> bool {
        HW_CPU_ID_MAP.lock().len() == num_cpus
    }

    while !is_all_aps_started(num_cpus) {
        core::hint::spin_loop();
    }
}

/// Constructs a boxed slice that maps [`CpuId`] to [`HwCpuId`].
///
/// # Panics
///
/// This method will panic if it is called either before all APs have booted or more than once.
///
/// [`CpuId`]: crate::cpu::CpuId
pub(crate) fn construct_hw_cpu_id_mapping() -> Box<[HwCpuId]> {
    let mut hw_cpu_id_map = HW_CPU_ID_MAP.lock();
    assert_eq!(hw_cpu_id_map.len(), crate::cpu::num_cpus());

    let result = hw_cpu_id_map
        .values()
        .cloned()
        .collect::<Vec<_>>()
        .into_boxed_slice();
    hw_cpu_id_map.clear();

    result
}
