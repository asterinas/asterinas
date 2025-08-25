// SPDX-License-Identifier: MPL-2.0

//! Multiprocessor Boot Support

use core::arch::global_asm;

use crate::{
    boot::smp::PerApRawInfo,
    cpu_local_cell,
    mm::{Paddr, Vaddr},
};

// Include the AP boot assembly code
global_asm!(include_str!("ap_boot.S"));

pub(crate) fn count_processors() -> Option<u32> {
    let mut hart_count = 0;

    for_each_hart_id(|_| hart_count += 1);

    if hart_count == 0 {
        None
    } else {
        Some(hart_count)
    }
}

/// Brings up all application processors.
///
/// Following the x86 naming, all the harts that are not the bootstrapping hart
/// are "application processors".
///
/// # Safety
///
/// The caller must ensure that
///  1. we're in the boot context of the BSP,
///  2. all APs have not yet been booted, and
///  3. the arguments are valid to boot APs.
pub(crate) unsafe fn bringup_all_aps(info_ptr: *const PerApRawInfo, pt_ptr: Paddr, num_cpus: u32) {
    if num_cpus <= 1 {
        return; // No APs to bring up
    }

    // SAFETY: We ensure that the variables are properly defined in the assembly
    // code and they are safe to write here.
    unsafe {
        fill_boot_info_ptr(info_ptr);
        fill_boot_page_table_ptr(pt_ptr);
    }

    let bsp_id = get_bootstrap_hart_id();

    log::info!("Bootstrapping hart is {}, booting all other harts", bsp_id);

    for_each_hart_id(|hart_id| {
        if hart_id != bsp_id {
            // SAFETY: Each hart ID is iterated over exactly once here so we
            // won't boot up one hart twice. Other safety constraints are
            // upheld by the caller.
            unsafe { bringup_ap(hart_id) };
        }
    });
}

fn for_each_hart_id(mut f: impl FnMut(u32)) {
    let Some(device_tree) = super::DEVICE_TREE.get() else {
        f(get_bootstrap_hart_id());
        return;
    };

    device_tree.cpus().for_each(|cpu_node| {
        if let Some(device_type) = cpu_node.property("device_type") {
            if device_type.as_str() == Some("cpu") {
                if let Some(reg) = cpu_node.property("reg") {
                    f(reg.as_usize().unwrap() as u32);
                }
            }
        }
    })
}

/// # Safety
///
/// The caller must ensure that
///  1. we're in the boot context of the BSP,
///  2. the resources for APs' boot (e.g., stacks and the page table) are set
///     up correctly, and
///  3. the `hart_id` hart hasn't booted.
unsafe fn bringup_ap(hart_id: u32) {
    log::info!("Starting hart {}", hart_id);

    // Use SBI to start the hart directly at the AP boot code
    let result = sbi_rt::hart_start(
        hart_id as usize,
        get_ap_boot_start_addr(),
        /* Unused */ 0,
    );

    if result.error == 0 {
        log::debug!("Successfully started hart {}", hart_id);
    } else {
        log::error!(
            "Failed to start hart {}: error code {}",
            hart_id,
            result.error
        );
    }
}

/// Fills the AP boot info array pointer.
///
/// # Safety
///
/// The caller must ensure that `__ap_boot_info_array_pointer` is safe to write.
unsafe fn fill_boot_info_ptr(info_ptr: *const PerApRawInfo) {
    extern "C" {
        static mut __ap_boot_info_array_pointer: *const PerApRawInfo;
    }

    // SAFETY: The safety conditions are upheld by the caller.
    unsafe {
        __ap_boot_info_array_pointer = info_ptr;
    }
}

/// Fills the AP boot page table pointer.
///
/// # Safety
///
/// The caller must ensure that `__ap_boot_page_table_pointer` is safe to write.
unsafe fn fill_boot_page_table_ptr(pt_ptr: Paddr) {
    extern "C" {
        static mut __ap_boot_page_table_pointer: Paddr;
    }

    // SAFETY: The safety conditions are upheld by the caller.
    unsafe {
        __ap_boot_page_table_pointer = pt_ptr;
    }
}

fn get_ap_boot_start_addr() -> Paddr {
    const KERNEL_VMA: Vaddr = 0xffffffff00000000;

    let addr: Paddr;

    // We need to load the address of the symbol in assembly to avoid the
    // linker relocation error. The symbol is not reachable using IP-offset
    // addressing without the virtual offset.
    unsafe {
        core::arch::asm!(
            "la {0}, ap_boot_start + {1}",
            out(reg) addr,
            const KERNEL_VMA,
        );
    }

    addr - KERNEL_VMA
}

fn get_bootstrap_hart_id() -> u32 {
    // SAFETY: This function is only called after `riscv_boot` has written the
    // ID. There must be a fence after the write if we are APs, and no other
    // writes happen after that so it's safe to read.
    unsafe { super::BOOTSTRAP_HART_ID }
}

pub(in crate::arch) fn get_current_hart_id() -> u32 {
    let id = AP_CURRENT_HART_ID.load();
    if id == u32::MAX {
        // This function cannot be called before `riscv_ap_early_entry`, which
        // is the entrypoint and initializes `AP_CURRENT_HART_ID`. So if the ID
        // is not written we must be the BSP.
        get_bootstrap_hart_id()
    } else {
        id
    }
}

cpu_local_cell! {
    static AP_CURRENT_HART_ID: u32 = u32::MAX;
}

// Since in RISC-V we cannot read the hart ID in S mode, the hart ID is
// delivered from the bootloader. We need to record the hart ID with another
// layer of entry point.
#[no_mangle]
unsafe extern "C" fn riscv_ap_early_entry(cpu_id: u32, hart_id: u32) -> ! {
    unsafe extern "C" {
        fn ap_early_entry(cpu_id: u32) -> !;
    }

    // CPU local memory could be accessed here since we are the AP and the BSP
    // must have initialized it.
    AP_CURRENT_HART_ID.store(hart_id);

    // SAFETY: This is valid to call and only called once.
    unsafe { ap_early_entry(cpu_id) };
}
