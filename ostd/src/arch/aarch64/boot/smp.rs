// SPDX-License-Identifier: MPL-2.0

//! Multiprocessor boot support via PSCI.

use core::arch::global_asm;

use crate::{boot::smp::PerApRawInfo, mm::Paddr};

global_asm!(include_str!("ap_boot.S"));

/// PSCI `CPU_ON` (SMC64) function identifier.
const PSCI_CPU_ON: u64 = 0xC400_0003;

/// Returns the number of processors described by the device tree.
pub(crate) fn count_processors() -> Option<u32> {
    let count = cpu_mpidrs().count() as u32;
    if count == 0 { Some(1) } else { Some(count) }
}

/// Iterates over the `reg` (MPIDR affinity) value of every CPU node in the
/// device tree, in device-tree order (index 0 is the BSP).
fn cpu_mpidrs() -> impl Iterator<Item = u64> {
    super::DEVICE_TREE.get().unwrap().cpus().filter_map(|cpu| {
        // A CPU node's `reg` holds the MPIDR affinity of that processor.
        cpu.property("reg").and_then(|reg| {
            let v = reg.value;
            match v.len() {
                4 => Some(u32::from_be_bytes(v.try_into().ok()?) as u64),
                8 => Some(u64::from_be_bytes(v.try_into().ok()?)),
                _ => None,
            }
        })
    })
}

/// Brings up all application processors via PSCI `CPU_ON`.
///
/// # Safety
///
/// The caller must ensure that we're in the boot context of the BSP, all APs
/// have not yet been booted, and the arguments are valid to boot APs.
pub(crate) unsafe fn bringup_all_aps(info_ptr: *const PerApRawInfo, pt_ptr: Paddr, num_cpus: u32) {
    if num_cpus <= 1 {
        return;
    }

    // SAFETY: The variables are defined in `ap_boot.S` and are safe to write
    // here before the APs are started.
    unsafe {
        fill_boot_info_ptr(info_ptr);
        fill_boot_page_table_ptr(pt_ptr);
    }

    let ap_entry = ap_boot_start_paddr();
    let mpidrs: alloc::vec::Vec<u64> = cpu_mpidrs().collect();

    // CPU index 0 is the BSP; indices 1..num_cpus are APs.
    for cpu_id in 1..num_cpus {
        let Some(&target) = mpidrs.get(cpu_id as usize) else {
            crate::error!("Missing device-tree CPU node for CPU {cpu_id}");
            continue;
        };

        // SAFETY: The entry point and per-AP resources are set up correctly, and
        // each CPU is started exactly once.
        let ret = unsafe { psci_cpu_on(target, ap_entry as u64, cpu_id as u64) };
        if ret != 0 {
            crate::error!("PSCI CPU_ON failed for CPU {cpu_id} (mpidr {target:#x}): {ret}");
        }
    }
}

/// Returns the physical entry address of the AP boot trampoline.
fn ap_boot_start_paddr() -> Paddr {
    unsafe extern "C" {
        fn ap_boot_start();
    }
    // `.ap_boot` is linked at physical (low) addresses, so the symbol address is
    // already the physical entry point.
    ap_boot_start as usize
}

/// Issues a PSCI `CPU_ON` call. Returns the PSCI status (0 = success).
///
/// # Safety
///
/// The caller must ensure the arguments describe a valid, not-yet-started CPU.
unsafe fn psci_cpu_on(target_cpu: u64, entry_point: u64, context_id: u64) -> i64 {
    let ret: i64;
    // SAFETY: PSCI calls have no memory-safety implications; the conduit is HVC
    // (the QEMU `virt` default when EL2 is present).
    unsafe {
        core::arch::asm!(
            "hvc #0",
            inout("x0") PSCI_CPU_ON => ret,
            in("x1") target_cpu,
            in("x2") entry_point,
            in("x3") context_id,
            options(nostack),
        );
    }
    ret
}

/// # Safety
///
/// The caller must ensure exclusive access to `__ap_boot_info_array_pointer`.
unsafe fn fill_boot_info_ptr(info_ptr: *const PerApRawInfo) {
    unsafe extern "C" {
        static mut __ap_boot_info_array_pointer: *const PerApRawInfo;
    }
    // SAFETY: The safety conditions are upheld by the caller.
    unsafe { __ap_boot_info_array_pointer = info_ptr };
}

/// # Safety
///
/// The caller must ensure exclusive access to `__ap_boot_page_table_pointer`.
unsafe fn fill_boot_page_table_ptr(pt_ptr: Paddr) {
    unsafe extern "C" {
        static mut __ap_boot_page_table_pointer: Paddr;
    }
    // SAFETY: The safety conditions are upheld by the caller.
    unsafe { __ap_boot_page_table_pointer = pt_ptr };
}

/// Returns the GIC CPU-interface number (`MPIDR_EL1.Aff0`) of the current CPU.
pub(in crate::arch) fn get_current_hart_id() -> u32 {
    let mpidr: u64;
    // SAFETY: Reading `MPIDR_EL1` has no side effects.
    unsafe { core::arch::asm!("mrs {}, mpidr_el1", out(reg) mpidr, options(nostack, nomem)) };
    (mpidr & 0xff) as u32
}
