// SPDX-License-Identifier: MPL-2.0

use linux_boot_params::BootParams;
use uefi::{boot::exit_boot_services, mem::memory_map::MemoryMap, prelude::*};
use uefi_raw::table::system::SystemTable;

use super::decoder::decode_payload;

const PAGE_SIZE: u64 = 4096;

#[export_name = "main_efi_handover64"]
extern "sysv64" fn main_efi_handover64(
    handle: Handle,
    system_table: *const SystemTable,
    boot_params_ptr: *mut BootParams,
) -> ! {
    // SAFETY: We get `handle` and `system_table` from the UEFI firmware, so by contract the
    // pointers are valid and correct.
    unsafe {
        boot::set_image_handle(handle);
        uefi::table::set_system_table(system_table);
    }

    uefi::helpers::init().unwrap();

    // SAFETY: We get boot parameters from the boot loader, so by contract the pointer is valid and
    // the underlying memory is initialized. We are an exclusive owner of the memory region, so we
    // can create a mutable reference of the plain-old-data type.
    let boot_params = unsafe { &mut *boot_params_ptr };

    efi_phase_boot(boot_params);

    // SAFETY: We do not open boot service protocols or maintain references to boot service code
    // and data.
    unsafe { efi_phase_runtime(boot_params) };
}

fn efi_phase_boot(boot_params: &mut BootParams) {
    uefi::println!(
        "[EFI stub] Loaded with offset {:#x}",
        crate::x86::image_load_offset(),
    );

    // Fill the boot params with the RSDP address if it is not provided.
    if boot_params.acpi_rsdp_addr == 0 {
        boot_params.acpi_rsdp_addr =
            find_rsdp_addr().expect("ACPI RSDP address is not available") as usize as u64;
    }

    // Decode the payload and load it as an ELF file.
    uefi::println!("[EFI stub] Decoding the kernel payload");
    let kernel = decode_payload(crate::x86::payload());
    uefi::println!("[EFI stub] Loading the payload as an ELF file");
    crate::loader::load_elf(&kernel);
}

fn find_rsdp_addr() -> Option<*const ()> {
    use uefi::table::cfg::{ACPI2_GUID, ACPI_GUID};

    // Prefer ACPI2 over ACPI.
    for acpi_guid in [ACPI2_GUID, ACPI_GUID] {
        if let Some(rsdp_addr) = uefi::system::with_config_table(|table| {
            table
                .iter()
                .find(|entry| entry.guid == acpi_guid)
                .map(|entry| entry.address.cast::<()>())
        }) {
            return Some(rsdp_addr);
        }
    }

    None
}

unsafe fn efi_phase_runtime(boot_params: &mut BootParams) -> ! {
    uefi::println!("[EFI stub] Exiting EFI boot services");
    // SAFETY: The safety is upheld by the caller.
    let memory_map = unsafe { exit_boot_services(uefi::table::boot::MemoryType::LOADER_DATA) };

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

    // Write the memory map to the E820 table in `boot_params`.
    let e820_table = &mut boot_params.e820_table;
    let mut num_entries = 0usize;
    for entry in memory_map.entries() {
        let typ = if let Some(e820_type) = parse_memory_type(entry.ty) {
            e820_type
        } else {
            // The memory region is unaccepted (i.e., `MemoryType::UNACCEPTED`).
            crate::println!("[EFI stub] Accepting pending pages");
            for page_idx in 0..entry.page_count {
                // SAFETY: The page to accept represents a page that has not been accepted
                // (according to the memory map returned by the UEFI firmware).
                unsafe {
                    tdx_guest::tdcall::accept_page(0, entry.phys_start + page_idx * PAGE_SIZE)
                        .unwrap();
                }
            }
            linux_boot_params::E820Type::Ram
        };

        if num_entries != 0 {
            let last_entry = &mut e820_table[num_entries - 1];
            let last_typ = last_entry.typ;
            if last_typ == typ && last_entry.addr + last_entry.size == entry.phys_start {
                last_entry.size += entry.page_count * PAGE_SIZE;
                continue;
            }
        }

        if num_entries >= e820_table.len() {
            crate::println!("[EFI stub] Warning: The number of E820 entries exceeded 128!");
            break;
        }

        e820_table[num_entries] = linux_boot_params::BootE820Entry {
            addr: entry.phys_start,
            size: entry.page_count * PAGE_SIZE,
            typ,
        };
        num_entries += 1;
    }
    boot_params.e820_entries = num_entries as u8;

    crate::println!(
        "[EFI stub] Entering the Asterinas entry point at {:p}",
        super::ASTER_ENTRY_POINT,
    );
    // SAFETY:
    // 1. The entry point address is correct and matches the kernel ELF file.
    // 2. The boot parameter pointer is valid and points to the correct boot parameters.
    unsafe { super::call_aster_entrypoint(super::ASTER_ENTRY_POINT, boot_params) }
}

fn parse_memory_type(
    mem_type: uefi::table::boot::MemoryType,
) -> Option<linux_boot_params::E820Type> {
    use linux_boot_params::E820Type;
    use uefi::table::boot::MemoryType;

    match mem_type {
        // UEFI Specification, 7.2 Memory Allocation Services:
        //   Following the ExitBootServices() call, the image implicitly owns all unused memory in
        //   the map. This includes memory types EfiLoaderCode, EfiLoaderData, EfiBootServicesCode,
        //   EfiBootServicesData, and EfiConventionalMemory
        //
        // Note that this includes the loaded kernel! The kernel itself should take care of this.
        //
        // TODO: Linux takes memory attributes into account. See
        // <https://github.com/torvalds/linux/blob/b7f94fcf55469ad3ef8a74c35b488dbfa314d1bb/arch/x86/platform/efi/efi.c#L133-L139>.
        MemoryType::LOADER_CODE
        | MemoryType::LOADER_DATA
        | MemoryType::BOOT_SERVICES_CODE
        | MemoryType::BOOT_SERVICES_DATA
        | MemoryType::CONVENTIONAL => Some(E820Type::Ram),

        // Some memory types have special meanings.
        MemoryType::PERSISTENT_MEMORY => Some(E820Type::Pmem),
        MemoryType::ACPI_RECLAIM => Some(E820Type::Acpi),
        MemoryType::ACPI_NON_VOLATILE => Some(E820Type::Nvs),
        MemoryType::UNUSABLE => Some(E820Type::Unusable),
        MemoryType::UNACCEPTED => None,

        // Other memory types are treated as reserved.
        MemoryType::RESERVED
        | MemoryType::RUNTIME_SERVICES_CODE
        | MemoryType::RUNTIME_SERVICES_DATA
        | MemoryType::MMIO
        | MemoryType::MMIO_PORT_SPACE => Some(E820Type::Reserved),
        _ => Some(E820Type::Reserved),
    }
}
