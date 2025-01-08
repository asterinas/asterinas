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

use crate::{
    arch::x86::kernel::{
        acpi::get_acpi_tables,
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
    let acpi_tables = get_acpi_tables()?;
    let mut local_apic_counts = 0;
    acpi_tables
        .find_table::<acpi::madt::Madt>()
        .unwrap()
        .entries()
        .for_each(|entry| {
            if let acpi::madt::MadtEntry::LocalApic(_) = entry {
                local_apic_counts += 1;
            }
        });

    Some(local_apic_counts)
}

/// Brings up all application processors.
pub(crate) fn bringup_all_aps() {
    copy_ap_boot_code();
    fill_boot_stack_array_ptr();
    fill_boot_pt_ptr();
    send_boot_ipis();
}

/// This is where the linker load the symbols in the `.ap_boot` section.
/// The BSP would copy the AP boot code to this address.
pub(super) const AP_BOOT_START_PA: usize = 0x8000;

/// The size of the AP boot code (the `.ap_boot` section).
pub(super) fn ap_boot_code_size() -> usize {
    __ap_boot_end as usize - __ap_boot_start as usize
}

fn copy_ap_boot_code() {
    let ap_boot_start = __ap_boot_start as usize as *const u8;
    let len = __ap_boot_end as usize - __ap_boot_start as usize;

    // SAFETY: we are copying the AP boot code to the AP boot address.
    unsafe {
        core::ptr::copy_nonoverlapping(
            ap_boot_start,
            crate::mm::paddr_to_vaddr(AP_BOOT_START_PA) as *mut u8,
            len,
        );
    }
}

/// Initializes the boot stack array in the AP boot code with the given pages.
fn fill_boot_stack_array_ptr() {
    let pages = &crate::boot::smp::AP_BOOT_INFO
        .get()
        .unwrap()
        .boot_stack_array;

    extern "C" {
        static __ap_boot_stack_array_pointer: u64;
    }

    // SAFETY: This pointer points to a static variable defined in the `ap_boot.S`.
    let ptr = unsafe { &__ap_boot_stack_array_pointer as *const u64 as *mut u64 };
    // SAFETY: We only write to it once.
    unsafe {
        ptr.write_volatile(paddr_to_vaddr(pages.start_paddr()) as u64);
    }
}

fn fill_boot_pt_ptr() {
    extern "C" {
        static __boot_page_table_pointer: u32;
    }
    let boot_pt = crate::mm::page_table::boot_pt::with_borrow(|pt| pt.root_address()).unwrap();

    // SAFETY: this pointer points to a static variable defined in the `ap_boot.S`.
    let ptr = unsafe { &__boot_page_table_pointer as *const u32 as *mut u32 };
    // SAFETY: We only write to it once.
    unsafe {
        ptr.write_volatile(boot_pt as u32);
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
fn send_boot_ipis() {
    send_init_to_all_aps();

    spin_wait_cycles(100_000_000);

    send_init_deassert();

    spin_wait_cycles(20_000_000);

    send_startup_to_all_aps();

    spin_wait_cycles(20_000_000);

    send_startup_to_all_aps();

    spin_wait_cycles(20_000_000);
}

fn send_startup_to_all_aps() {
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
    // SAFETY: we are sending startup IPI to all APs.
    apic::with_borrow(|apic| unsafe { apic.send_ipi(icr) });
}

fn send_init_to_all_aps() {
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
    // SAFETY: we are sending init IPI to all APs.
    apic::with_borrow(|apic| unsafe { apic.send_ipi(icr) });
}

fn send_init_deassert() {
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
    // SAFETY: we are sending deassert IPI to all APs.
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

    let start = unsafe { _rdtsc() };

    while duration(start, unsafe { _rdtsc() }) < c {
        core::hint::spin_loop();
    }
}
