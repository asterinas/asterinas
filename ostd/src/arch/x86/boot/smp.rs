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

use acpi::madt::MadtEntry;

use crate::{
    arch::{
        if_tdx_enabled,
        kernel::{
            acpi::get_acpi_tables,
            apic::{
                self, Apic, ApicId, DeliveryMode, DeliveryStatus, DestinationMode,
                DestinationShorthand, Icr, Level, TriggerMode,
            },
        },
    },
    boot::{
        memory_region::{MemoryRegion, MemoryRegionType},
        smp::PerApRawInfo,
    },
    mm::{Paddr, PAGE_SIZE},
    task::disable_preempt,
};

/// Counts the number of processors.
///
/// This function needs to be called after the OS initializes the ACPI table.
pub(crate) fn count_processors() -> Option<u32> {
    let acpi_tables = get_acpi_tables()?;
    let madt_table = acpi_tables.find_table::<acpi::madt::Madt>().ok()?;

    // According to ACPI spec [1], "If this bit [the Enabled bit] is set the processor is ready for
    // use. If this bit is clear and the Online Capable bit is set, system hardware supports
    // enabling this processor during OS runtime."
    // [1]: https://uefi.org/htmlspecs/ACPI_Spec_6_4_html/05_ACPI_Software_Programming_Model/ACPI_Software_Programming_Model.html#local-apic-flags
    fn is_usable(flags: u32) -> bool {
        const ENABLED: u32 = 0b01;
        const ONLINE_CAPABLE: u32 = 0b10;

        (flags & ENABLED) != 0 || (flags & ONLINE_CAPABLE) != 0
    }

    // According to ACPI spec [1], "Logical processors with APIC ID values less than 255 (whether
    // in XAPIC or X2APIC mode) must use the Processor Local APIC structure to convey their APIC
    // information to OSPM [..] Logical processors with APIC ID values 255 and greater must use the
    // Processor Local x2APIC structure [..]"
    // [1]: https://uefi.org/htmlspecs/ACPI_Spec_6_4_html/05_ACPI_Software_Programming_Model/ACPI_Software_Programming_Model.html#processor-local-x2apic-structure
    let is_dup_apic = |id: u32| -> bool {
        // Check if the APIC entry also shows up as an x2APIC entry.
        if madt_table.get().entries().any(|e| {
            matches!(e, MadtEntry::LocalX2Apic(e)
                if e.x2apic_id == id && is_usable(e.flags))
        }) {
            log::warn!(
                "Firmware bug: In MADT, APIC ID {} is also listed as an x2APIC ID",
                id,
            );
            true
        } else {
            false
        }
    };

    let local_apic_counts = madt_table
        .get()
        .entries()
        .filter(|e| match e {
            MadtEntry::LocalX2Apic(entry) => {
                log::trace!("Found a local x2APIC entry in MADT: {:?}", entry);
                is_usable(entry.flags)
            }
            MadtEntry::LocalApic(entry) => {
                log::trace!("Found a local APIC entry in MADT: {:?}", entry);
                is_usable(entry.flags) && !is_dup_apic(entry.apic_id as u32)
            }
            _ => false,
        })
        .count();

    Some(local_apic_counts as u32)
}

/// Brings up all application processors.
///
/// # Safety
///
/// The caller must ensure that
/// 1. we're in the boot context of the BSP,
/// 2. all APs have not yet been booted, and
/// 3. the arguments are valid to boot APs.
pub(crate) unsafe fn bringup_all_aps(info_ptr: *const PerApRawInfo, pt_ptr: Paddr, num_cpus: u32) {
    // SAFETY: The code and data to boot AP is valid to write because
    // there are no readers and we are the only writer at this point.
    unsafe {
        copy_ap_boot_code();
        fill_boot_info_ptr(info_ptr);
        fill_boot_pt_ptr(pt_ptr);
    }

    // SAFETY: We've properly prepared all the resources to boot APs.
    if_tdx_enabled!({
        unsafe { wake_up_aps_via_mailbox(num_cpus) };
    } else {
        unsafe { send_boot_ipis() };
    });
}

/// This is where the linker load the symbols in the `.ap_boot` section.
/// The BSP would copy the AP boot code to this address.
const AP_BOOT_START_PA: usize = 0x8000;

/// The size of the AP boot code (the `.ap_boot` section).
fn ap_boot_code_size() -> usize {
    __ap_boot_end as usize - __ap_boot_start as usize
}

pub(super) fn reclaimable_memory_region() -> MemoryRegion {
    MemoryRegion::new(
        AP_BOOT_START_PA,
        ap_boot_code_size(),
        MemoryRegionType::Reclaimable,
    )
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
unsafe fn fill_boot_info_ptr(info_ptr: *const PerApRawInfo) {
    extern "C" {
        static mut __ap_boot_info_array_pointer: *const PerApRawInfo;
    }

    // SAFETY: The safety is upheld by the caller.
    unsafe {
        __ap_boot_info_array_pointer = info_ptr;
    }
}

/// # Safety
///
/// The caller must ensure the pointer to be filled is valid to write.
unsafe fn fill_boot_pt_ptr(pt_ptr: Paddr) {
    extern "C" {
        static mut __boot_page_table_pointer: u32;
    }

    let pt_ptr32 = pt_ptr.try_into().unwrap();

    // SAFETY: The safety is upheld by the caller.
    unsafe {
        __boot_page_table_pointer = pt_ptr32;
    }
}

// The symbols are defined in the linker script.
extern "C" {
    fn __ap_boot_start();
    fn __ap_boot_end();
}

/// Wakes up all application processors via the ACPI multiprocessor mailbox structure.
///
/// # Safety
///
/// The safety preconditions are the same as [`send_boot_ipis`].
#[cfg(feature = "cvm_guest")]
unsafe fn wake_up_aps_via_mailbox(num_cpus: u32) {
    use acpi::platform::wakeup_aps;

    use crate::arch::kernel::acpi::AcpiMemoryHandler;

    // The symbols are defined in `ap_boot.S`.
    extern "C" {
        fn ap_boot_from_real_mode();
        fn ap_boot_from_long_mode();
    }

    let offset = ap_boot_from_long_mode as usize - ap_boot_from_real_mode as usize;

    let acpi_tables = get_acpi_tables().unwrap();
    for ap_num in 1..num_cpus {
        wakeup_aps(
            &acpi_tables,
            AcpiMemoryHandler {},
            ap_num,
            (AP_BOOT_START_PA + offset) as u64,
            1000,
        )
        .unwrap();
    }
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
    let preempt_guard = disable_preempt();
    let apic = apic::get_or_init(&preempt_guard as _);

    // SAFETY: We're sending IPIs to boot all application processors.
    // The safety is upheld by the caller.
    unsafe {
        send_init_to_all_aps(apic);
        spin_wait_cycles(100_000_000);

        send_init_deassert(apic);
        spin_wait_cycles(20_000_000);

        send_startup_to_all_aps(apic);
        spin_wait_cycles(20_000_000);

        send_startup_to_all_aps(apic);
        spin_wait_cycles(20_000_000);
    }
}

/// # Safety
///
/// The caller should ensure it's valid to send STARTUP IPIs to all CPUs excluding self.
unsafe fn send_startup_to_all_aps(apic: &dyn Apic) {
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
    unsafe { apic.send_ipi(icr) }
}

/// # Safety
///
/// The caller should ensure it's valid to send INIT IPIs to all CPUs excluding self.
unsafe fn send_init_to_all_aps(apic: &dyn Apic) {
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
    unsafe { apic.send_ipi(icr) };
}

/// # Safety
///
/// The caller should ensure it's valid to deassert INIT IPIs for all CPUs excluding self.
unsafe fn send_init_deassert(apic: &dyn Apic) {
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
    unsafe { apic.send_ipi(icr) };
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

    let start = crate::arch::read_tsc();
    while duration(start, crate::arch::read_tsc()) < c {
        core::hint::spin_loop();
    }
}
