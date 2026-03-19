// SPDX-License-Identifier: MPL-2.0

use linux_boot_params::BootParams;
use uefi_raw::{
    Guid,
    table::{Header, system::SystemTable},
};

pub(super) fn fill_acpi_rsdp_addr(boot_params: &mut BootParams) {
    if boot_params.acpi_rsdp_addr != 0 {
        return;
    }

    let Some(rsdp_addr) = find_rsdp_addr(boot_params) else {
        return;
    };

    boot_params.acpi_rsdp_addr = rsdp_addr;
    crate::println!("[setup] Found the ACPI RSDP at {:#x}", rsdp_addr);
}

const EFI64_LOADER_SIGNATURE: u32 = u32::from_le_bytes(*b"EL64");
const MAX_EFI_CONFIG_TABLES: usize = 1024;
const ACPI_GUID: Guid = uefi_raw::guid!("eb9d2d30-2d88-11d3-9a16-0090273fc14d");
const ACPI2_GUID: Guid = uefi_raw::guid!("8868e871-e4f1-11d3-bc22-0080c73c8881");

fn find_rsdp_addr(boot_params: &BootParams) -> Option<u64> {
    let efi = boot_params.efi_info;

    let systab_addr = (efi.efi_systab as u64) | ((efi.efi_systab_hi as u64) << 32);
    if systab_addr == 0 {
        crate::println!("[setup] Warning: No EFI system table address is provided in boot params!");
        return None;
    }

    // In legacy 32-bit setup code, pointers above 4GiB are not addressable.
    let systab_addr = usize::try_from(systab_addr).ok()?;

    match efi.efi_loader_signature {
        EFI64_LOADER_SIGNATURE => {
            // SAFETY: The system table address is provided by the bootloader via Linux boot params.
            unsafe { find_rsdp_addr_in_efi64(systab_addr as *const EfiSystemTable64) }
        }
        _ => None,
    }
}

/// Represents the 64-bit UEFI system table layout carried by Linux boot params.
///
/// The legacy i386 setup code may still receive an `EL64` loader signature,
/// meaning `efi_systab` points to a 64-bit UEFI `SystemTable`. We define this
/// local type because `uefi_raw::table::system::SystemTable` follows the target
/// pointer width and therefore models only the 32-bit layout on this build
/// target.
#[repr(C)]
struct EfiSystemTable64 {
    header: Header,
    firmware_vendor: u64,
    firmware_revision: u32,
    // Remember that we are 32 bit, so the padding is needed.
    _padding: u32,
    stdin_handle: u64,
    stdin: u64,
    stdout_handle: u64,
    stdout: u64,
    stderr_handle: u64,
    stderr: u64,
    runtime_services: u64,
    boot_services: u64,
    number_of_configuration_table_entries: u64,
    configuration_table: u64,
}

/// Represents one entry in a 64-bit UEFI configuration table array.
#[repr(C)]
struct EfiConfigurationTable64 {
    vendor_guid: Guid,
    vendor_table: u64,
}

unsafe fn find_rsdp_addr_in_efi64(system_table_ptr: *const EfiSystemTable64) -> Option<u64> {
    if system_table_ptr.is_null() {
        return None;
    }

    // SAFETY: The caller guarantees that the pointer originates from boot params.
    let system_table = unsafe { &*system_table_ptr };
    if system_table.header.signature != SystemTable::SIGNATURE {
        return None;
    }

    let nr_tables = usize::try_from(system_table.number_of_configuration_table_entries).ok()?;
    if nr_tables == 0 || nr_tables > MAX_EFI_CONFIG_TABLES {
        return None;
    }

    let tables_addr = usize::try_from(system_table.configuration_table).ok()?;
    if tables_addr == 0 {
        return None;
    }
    let tables = tables_addr as *const EfiConfigurationTable64;

    // SAFETY: The caller guarantees that the system table and config table pointers are valid.
    unsafe { find_in_cfg_tables64(tables, nr_tables) }
}

unsafe fn find_in_cfg_tables64(
    tables: *const EfiConfigurationTable64,
    nr_tables: usize,
) -> Option<u64> {
    // Prefer ACPI2 over ACPI, same as the x64 EFI path.
    for acpi_guid in [ACPI2_GUID, ACPI_GUID] {
        for index in 0..nr_tables {
            // SAFETY: `index` is bounded by `nr_tables`, and the caller guarantees table validity.
            let entry = unsafe { &*tables.add(index) };
            if entry.vendor_guid == acpi_guid {
                return Some(entry.vendor_table);
            }
        }
    }

    None
}
