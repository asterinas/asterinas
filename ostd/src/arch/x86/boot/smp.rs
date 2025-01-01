// SPDX-License-Identifier: MPL-2.0

//! Multiprocessor Boot Support
//!
//! The MP initialization protocol defines two classes of processors:
//! the bootstrap processor (BSP) and the application processors (APs).
//! Following a power-up or RESET of an MP system, system hardware dynamically
//! selects one of the processors on the system bus as the BSP. The remaining
//! processors are designated as APs.
//!
//! The BSP executes the BIOS's boot-strap code to configure the APIC environment,
//! sets up system-wide data structures. Up to now, BSP has completed most of the
//! initialization of the OS, but APs has not been awakened.
//!
//! Following a power-up or reset, the APs complete a minimal self-configuration,
//! then wait for a startup signal (a SIPI message) from the BSP processor.
//!
//! The wake-up of AP follows SNIT-SIPI-SIPI IPI sequence:
//!  - Broadcast INIT IPI (Initialize the APs to the wait-for-SIPI state)
//!  - Wait
//!  - Broadcast De-assert INIT IPI (Only older processors need this step)
//!  - Wait
//!  - Broadcast SIPI IPI (APs exits the wait-for-SIPI state and starts executing code)
//!  - Wait
//!  - Broadcast SIPI IPI (If an AP fails to start)
//!
//! This sequence does not need to be strictly followed, and there may be
//! different considerations in different systems.

use acpi::platform::PlatformInfo;

use crate::{
    arch::x86::kernel::{
        acpi::ACPI_TABLES,
        apic::{
            self, ApicId, DeliveryMode, DeliveryStatus, DestinationMode, DestinationShorthand, Icr,
            Level, TriggerMode,
        },
    },
    mm::{paddr_to_vaddr, PAGE_SIZE},
};

/// Get the number of processors
///
/// This function needs to be called after the OS initializes the ACPI table.
pub(crate) fn get_num_processors() -> Option<u32> {
    if !ACPI_TABLES.is_completed() {
        return None;
    }
    let processor_info = PlatformInfo::new(&*ACPI_TABLES.get().unwrap().lock())
        .unwrap()
        .processor_info
        .unwrap();
    Some(processor_info.application_processors.len() as u32 + 1)
}

/// Brings up all application processors.
///
/// # Safety
///
/// The caller must ensure that
/// 1. we're in the boot context of the BSP, and
/// 2. all APs have not yet been booted.
pub(crate) unsafe fn bringup_all_aps() {
    // SAFETY: The code and data to boot AP is valid to write because
    // there are no readers and we are the only writer at this point.
    unsafe {
        copy_ap_boot_code();
        fill_boot_stack_array_ptr();
        fill_boot_pt_ptr();
    }

    // SAFETY: We've properly prepared all the resources to boot APs.
    unsafe { send_boot_ipis() };
}

/// This is where the linker load the symbols in the `.ap_boot` section.
/// The BSP would copy the AP boot code to this address.
pub(super) const AP_BOOT_START_PA: usize = 0x8000;

/// The size of the AP boot code (the `.ap_boot` section).
pub(super) fn ap_boot_code_size() -> usize {
    __ap_boot_end as usize - __ap_boot_start as usize
}

/// # Safety
///
/// The caller must ensure the memory region to be filled with AP boot code is valid to write.
unsafe fn copy_ap_boot_code() {
    let ap_boot_start = __ap_boot_start as usize as *const u8;
    let len = __ap_boot_end as usize - __ap_boot_start as usize;

    // SAFETY:
    // 1. The source memory region is valid for reading because it's inside the kernel text.
    // 2. The destination memory region is valid for writing because the caller upholds this.
    // 3. The memory is aligned because the alignment of `u8` is 1.
    // 4. The two memory regions do not overlap because the kernel text is isolated with the AP
    //    boot region.
    unsafe {
        core::ptr::copy_nonoverlapping(
            ap_boot_start,
            crate::mm::paddr_to_vaddr(AP_BOOT_START_PA) as *mut u8,
            len,
        );
    }
}

/// # Safety
///
/// The caller must ensure the pointer to be filled is valid to write.
unsafe fn fill_boot_stack_array_ptr() {
    let pages = &crate::boot::smp::AP_BOOT_INFO
        .get()
        .unwrap()
        .boot_stack_array;
    let vaddr = paddr_to_vaddr(pages.start_paddr());

    extern "C" {
        static mut __ap_boot_stack_array_pointer: usize;
    }

    // SAFETY: The safety is upheld by the caller.
    unsafe {
        __ap_boot_stack_array_pointer = vaddr;
    }
}

/// # Safety
///
/// The caller must ensure the pointer to be filled is valid to write.
unsafe fn fill_boot_pt_ptr() {
    extern "C" {
        static mut __boot_page_table_pointer: u32;
    }

    let boot_pt = crate::mm::page_table::boot_pt::with_borrow(|pt| pt.root_address())
        .unwrap()
        .try_into()
        .unwrap();

    // SAFETY: The safety is upheld by the caller.
    unsafe {
        __boot_page_table_pointer = boot_pt;
    }
}

// The symbols are defined in the linker script.
extern "C" {
    fn __ap_boot_start();
    fn __ap_boot_end();
}

/// Sends IPIs to notify all application processors to boot.
///
/// Follow the INIT-SIPI-SIPI IPI sequence.
/// Here, we don't check whether there is an AP that failed to start,
/// but send the second SIPI directly (checking whether each core is
/// started successfully one by one will bring extra overhead). For
/// APs that have been started, this signal will not bring any cost.
///
/// # Safety
///
/// The caller must ensure that all application processors can be
/// safely booted by ensuring that:
/// 1. We're in the boot context of the BSP and all APs have not yet
///    been booted.
/// 2. We've properly prepared all the resources for the application
///    processors to boot successfully (e.g., each AP's page table
///    and stack).
unsafe fn send_boot_ipis() {
    // SAFETY: We're sending IPIs to boot all application processors.
    // The safety is upheld by the caller.
    unsafe {
        send_init_to_all_aps();
        spin_wait_cycles(100_000_000);

        send_init_deassert();
        spin_wait_cycles(20_000_000);

        send_startup_to_all_aps();
        spin_wait_cycles(20_000_000);

        send_startup_to_all_aps();
        spin_wait_cycles(20_000_000);
    }
}

/// # Safety
///
/// The caller should ensure it's valid to send STARTUP IPIs to all CPUs excluding self.
unsafe fn send_startup_to_all_aps() {
    let icr = Icr::new(
        ApicId::from(0),
        DestinationShorthand::AllExcludingSelf,
        TriggerMode::Edge,
        Level::Assert,
        DeliveryStatus::Idle,
        DestinationMode::Physical,
        DeliveryMode::StartUp,
        (AP_BOOT_START_PA / PAGE_SIZE) as u8,
    );
    // SAFETY: The safety is upheld by the caller.
    apic::with_borrow(|apic| unsafe { apic.send_ipi(icr) });
}

/// # Safety
///
/// The caller should ensure it's valid to send INIT IPIs to all CPUs excluding self.
unsafe fn send_init_to_all_aps() {
    let icr = Icr::new(
        ApicId::from(0),
        DestinationShorthand::AllExcludingSelf,
        TriggerMode::Level,
        Level::Assert,
        DeliveryStatus::Idle,
        DestinationMode::Physical,
        DeliveryMode::Init,
        0,
    );
    // SAFETY: The safety is upheld by the caller.
    apic::with_borrow(|apic| unsafe { apic.send_ipi(icr) });
}

/// # Safety
///
/// The caller should ensure it's valid to deassert INIT IPIs for all CPUs excluding self.
unsafe fn send_init_deassert() {
    let icr = Icr::new(
        ApicId::from(0),
        DestinationShorthand::AllIncludingSelf,
        TriggerMode::Level,
        Level::Deassert,
        DeliveryStatus::Idle,
        DestinationMode::Physical,
        DeliveryMode::Init,
        0,
    );
    // SAFETY: The safety is upheld by the caller.
    apic::with_borrow(|apic| unsafe { apic.send_ipi(icr) });
}

/// Spin wait approximately `c` cycles.
///
/// Since the timer requires CPU local storage to be initialized, we
/// can only wait by spinning.
fn spin_wait_cycles(c: u64) {
    fn duration(from: u64, to: u64) -> u64 {
        if to >= from {
            to - from
        } else {
            u64::MAX - from + to
        }
    }

    use core::arch::x86_64::_rdtsc;

    // SAFETY: Reading CPU cycels is always safe.
    let start = unsafe { _rdtsc() };

    // SAFETY: Reading CPU cycels is always safe.
    while duration(start, unsafe { _rdtsc() }) < c {
        core::hint::spin_loop();
    }
}
