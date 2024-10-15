// SPDX-License-Identifier: MPL-2.0

use linux_boot_params::BootParams;
use uefi::{
    boot::{exit_boot_services, open_protocol_exclusive},
    mem::memory_map::{MemoryMap, MemoryMapOwned},
    prelude::*,
    proto::loaded_image::LoadedImage,
};
use uefi_raw::table::system::SystemTable;

use super::{
    decoder::decode_payload,
    paging::{Ia32eFlags, PageNumber, PageTableCreator},
    relocation::apply_rela_relocations,
};

// Suppress warnings since using todo!.
#[allow(unreachable_code)]
#[allow(unused_variables)]
#[allow(clippy::diverging_sub_expression)]
#[export_name = "efi_stub_entry"]
extern "sysv64" fn efi_stub_entry(handle: Handle, system_table: *const SystemTable) -> ! {
    // SAFETY: handle and system_table are valid pointers. It is only called once.
    unsafe { system_init(handle, system_table) };

    uefi::helpers::init().unwrap();

    let boot_params = todo!("Use EFI boot services to fill boot params");

    efi_phase_boot(boot_params);
}

#[export_name = "efi_handover_entry"]
extern "sysv64" fn efi_handover_entry(
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
    // SAFETY: This is the right time to initialize the console and it is only
    // called once here before all console operations.
    unsafe {
        crate::console::init();
    }

    // SAFETY: This is the right time to apply relocations.
    unsafe { apply_rela_relocations() };

    // SAFETY: The handle and system_table are valid pointers. They are passed
    // from the UEFI firmware. They are only called once.
    unsafe {
        boot::set_image_handle(handle);
        uefi::table::set_system_table(system_table);
    }
}

fn efi_phase_boot(boot_params: &mut BootParams) -> ! {
    uefi::println!("[EFI stub] Relocations applied.");
    uefi::println!(
        "[EFI stub] Stub loaded at {:#x?}",
        crate::x86::get_image_loaded_offset()
    );

    // Fill the boot params with the RSDP address if it is not provided.
    if boot_params.acpi_rsdp_addr == 0 {
        boot_params.acpi_rsdp_addr = get_rsdp_addr();
    }

    // Load the kernel payload to memory.
    let payload = crate::get_payload(boot_params);
    let kernel = decode_payload(payload);

    uefi::println!("[EFI stub] Loading payload.");
    crate::loader::load_elf(&kernel);

    uefi::println!("[EFI stub] Exiting EFI boot services.");
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
    unsafe {
        crate::console::print_str("[EFI stub] Entered runtime services.\n");
    }

    #[cfg(feature = "debug_print")]
    unsafe {
        use crate::console::{print_hex, print_str};
        print_str("[EFI stub debug] EFI Memory map:\n");
        for md in memory_map.entries() {
            // crate::println!("    [{:#x}] {:#x} ({:#x})", md.ty.0, md.phys_start, md.page_count);
            print_str("    [");
            print_hex(md.ty.0 as u64);
            print_str("]");
            print_hex(md.phys_start);
            print_str("(size=");
            print_hex(md.page_count);
            print_str(")");
            print_str("{flags=");
            print_hex(md.att.bits());
            print_str("}\n");
        }
    }

    // Write memory map to e820 table in boot_params.
    let e820_table = &mut boot_params.e820_table;
    let mut e820_entries = 0usize;
    for md in memory_map.entries() {
        if e820_entries >= e820_table.len() || e820_entries >= 127 {
            unsafe {
                crate::console::print_str(
                    "[EFI stub] Warning: number of E820 entries exceeded 128!\n",
                );
            }
            break;
        }
        e820_table[e820_entries] = linux_boot_params::BootE820Entry {
            addr: md.phys_start,
            size: md.page_count * 4096,
            typ: match md.ty {
                uefi::table::boot::MemoryType::CONVENTIONAL => linux_boot_params::E820Type::Ram,
                uefi::table::boot::MemoryType::RESERVED => linux_boot_params::E820Type::Reserved,
                uefi::table::boot::MemoryType::ACPI_RECLAIM => linux_boot_params::E820Type::Acpi,
                uefi::table::boot::MemoryType::ACPI_NON_VOLATILE => {
                    linux_boot_params::E820Type::Nvs
                }
                _ => linux_boot_params::E820Type::Unusable,
            },
        };
        e820_entries += 1;
    }
    boot_params.e820_entries = e820_entries as u8;

    unsafe {
        crate::console::print_str("[EFI stub] Setting up the page table.\n");
    }

    // Make a new linear page table. The linear page table will be stored at
    // 0x4000000, hoping that the firmware will not use this area.
    let mut creator = unsafe {
        PageTableCreator::new(
            PageNumber::from_addr(0x4000000),
            PageNumber::from_addr(0x8000000),
        )
    };
    // Map the following regions:
    //  - 0x0: identity map the first 4GiB;
    //  - 0xffff8000_00000000: linear map 4GiB to low 4 GiB;
    //  - 0xffffffff_80000000: linear map 2GiB to low 2 GiB;
    //  - 0xffff8008_00000000: linear map 1GiB to 0x00000008_00000000.
    let flags = Ia32eFlags::PRESENT | Ia32eFlags::WRITABLE;
    for i in 0..4 * 1024 * 1024 * 1024 / 4096 {
        let from_vpn = PageNumber::from_addr(i * 4096);
        let from_vpn2 = PageNumber::from_addr(i * 4096 + 0xffff8000_00000000);
        let to_low_pfn = PageNumber::from_addr(i * 4096);
        creator.map(from_vpn, to_low_pfn, flags);
        creator.map(from_vpn2, to_low_pfn, flags);
    }
    for i in 0..2 * 1024 * 1024 * 1024 / 4096 {
        let from_vpn = PageNumber::from_addr(i * 4096 + 0xffffffff_80000000);
        let to_low_pfn = PageNumber::from_addr(i * 4096);
        creator.map(from_vpn, to_low_pfn, flags);
    }
    for i in 0..1024 * 1024 * 1024 / 4096 {
        let from_vpn = PageNumber::from_addr(i * 4096 + 0xffff8008_00000000);
        let to_pfn = PageNumber::from_addr(i * 4096 + 0x00000008_00000000);
        creator.map(from_vpn, to_pfn, flags);
    }
    // Mark this as reserved in e820 table.
    e820_table[e820_entries] = linux_boot_params::BootE820Entry {
        addr: 0x4000000,
        size: creator.nr_frames_used() as u64 * 4096,
        typ: linux_boot_params::E820Type::Reserved,
    };
    e820_entries += 1;
    boot_params.e820_entries = e820_entries as u8;

    #[cfg(feature = "debug_print")]
    unsafe {
        crate::console::print_str("[EFI stub] Activating the new page table.\n");
    }

    unsafe {
        creator.activate(x86_64::registers::control::Cr3Flags::PAGE_LEVEL_CACHE_DISABLE);
    }

    #[cfg(feature = "debug_print")]
    unsafe {
        crate::console::print_str("[EFI stub] Page table activated.\n");
    }

    unsafe {
        use crate::console::{print_hex, print_str};
        print_str("[EFI stub] Entering Asterinas entrypoint at ");
        print_hex(super::ASTER_ENTRY_POINT as u64);
        print_str("\n");
    }

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
