// SPDX-License-Identifier: MPL-2.0

use linux_boot_params::BootParams;
use uefi::{
    boot::{exit_boot_services, open_protocol_exclusive},
    mem::memory_map::{MemoryMap, MemoryMapOwned},
    prelude::*,
    proto::loaded_image::LoadedImage,
};
use uefi_raw::table::system::SystemTable;

use super::decoder::decode_payload;

const PAGE_SIZE: u64 = 4096;

#[export_name = "main_efi_handover64"]
extern "sysv64" fn main_efi_handover64(
    handle: Handle,
    system_table: *const SystemTable,
    boot_params_ptr: *mut BootParams,
) -> ! {
    // SAFETY: handle and system_table are valid pointers. It is only called once.
    unsafe { system_init(handle, system_table) };

    uefi::helpers::init().unwrap();

    // SAFETY: boot_params is a valid pointer.
    let boot_params = unsafe { &mut *boot_params_ptr };

    efi_phase_boot(boot_params)
}

/// Initialize the system.
///
/// # Safety
///
/// This function should be called only once with valid parameters before all
/// operations.
unsafe fn system_init(handle: Handle, system_table: *const SystemTable) {
    // SAFETY: The handle and system_table are valid pointers. They are passed
    // from the UEFI firmware. They are only called once.
    unsafe {
        boot::set_image_handle(handle);
        uefi::table::set_system_table(system_table);
    }
}

fn efi_phase_boot(boot_params: &mut BootParams) -> ! {
    uefi::println!(
        "[EFI stub] Loaded with offset {:#x}",
        crate::x86::image_load_offset(),
    );

    // Fill the boot params with the RSDP address if it is not provided.
    if boot_params.acpi_rsdp_addr == 0 {
        boot_params.acpi_rsdp_addr = get_rsdp_addr();
    }

    // Load the kernel payload to memory.
    let kernel = decode_payload(crate::x86::payload());

    uefi::println!("[EFI stub] Loading the payload as an ELF file");
    crate::loader::load_elf(&kernel);

    uefi::println!("[EFI stub] Exiting EFI boot services");
    let memory_type = {
        let Ok(loaded_image) = open_protocol_exclusive::<LoadedImage>(boot::image_handle()) else {
            panic!("Failed to open LoadedImage protocol");
        };
        loaded_image.data_type()
    };
    // SAFETY: All allocations in the boot services phase are not used after
    // this point.
    let memory_map = unsafe { exit_boot_services(memory_type) };

    efi_phase_runtime(memory_map, boot_params);
}

fn efi_phase_runtime(memory_map: MemoryMapOwned, boot_params: &mut BootParams) -> ! {
    crate::println!(
        "[EFI stub] Processing {} memory map entries",
        memory_map.entries().len()
    );
    #[cfg(feature = "debug_print")]
    {
        memory_map.entries().for_each(|entry| {
            crate::println!(
                "    [{:#x}] {:#x} (size={:#x}) {{flags={:#x}}}",
                entry.ty.0,
                entry.phys_start,
                entry.page_count,
                entry.att.bits()
            );
        })
    }

    // Write memory map to e820 table in boot_params.
    let e820_table = &mut boot_params.e820_table;
    let mut e820_entries = 0usize;
    for md in memory_map.entries() {
        if e820_entries >= e820_table.len() || e820_entries >= 127 {
            crate::println!("[EFI stub] Warning: The number of E820 entries exceeded 128!");
            break;
        }
        e820_table[e820_entries] = linux_boot_params::BootE820Entry {
            addr: md.phys_start,
            size: md.page_count * PAGE_SIZE,
            typ: match md.ty {
                uefi::table::boot::MemoryType::CONVENTIONAL => linux_boot_params::E820Type::Ram,
                uefi::table::boot::MemoryType::RESERVED => linux_boot_params::E820Type::Reserved,
                uefi::table::boot::MemoryType::ACPI_RECLAIM => linux_boot_params::E820Type::Acpi,
                uefi::table::boot::MemoryType::ACPI_NON_VOLATILE => {
                    linux_boot_params::E820Type::Nvs
                }
                #[cfg(feature = "cvm_guest")]
                uefi::table::boot::MemoryType::UNACCEPTED => {
                    unsafe {
                        crate::println!("[EFI stub] Accepting pending pages");
                        for page_idx in 0..md.page_count {
                            tdx_guest::tdcall::accept_page(0, md.phys_start + page_idx * PAGE_SIZE)
                                .unwrap();
                        }
                    };
                    linux_boot_params::E820Type::Ram
                }
                _ => linux_boot_params::E820Type::Unusable,
            },
        };
        e820_entries += 1;
    }
    boot_params.e820_entries = e820_entries as u8;

    crate::println!(
        "[EFI stub] Entering the Asterinas entry point at {:#x}",
        super::ASTER_ENTRY_POINT,
    );
    unsafe {
        super::call_aster_entrypoint(
            super::ASTER_ENTRY_POINT as u64,
            boot_params as *const _ as u64,
        )
    }
}

fn get_rsdp_addr() -> u64 {
    use uefi::table::cfg::{ACPI2_GUID, ACPI_GUID};
    uefi::system::with_config_table(|table| {
        for entry in table {
            // Prefer ACPI2 over ACPI.
            if entry.guid == ACPI2_GUID {
                return entry.address as usize as u64;
            }
            if entry.guid == ACPI_GUID {
                return entry.address as usize as u64;
            }
        }
        panic!("ACPI RSDP not found");
    })
}
