use uefi::{
    data_types::Handle,
    proto::loaded_image::LoadedImage,
    table::{boot::MemoryMap, Boot, Runtime, SystemTable},
};

use linux_boot_params::BootParams;

use super::paging::{Ia32eFlags, PageNumber, PageTableCreator};
use super::relocation::apply_rela_dyn_relocations;

#[export_name = "efi_stub_entry"]
extern "sysv64" fn efi_stub_entry(handle: Handle, mut system_table: SystemTable<Boot>) -> ! {
    unsafe {
        system_table.boot_services().set_image_handle(handle);
    }
    uefi_services::init(&mut system_table).unwrap();

    // Suppress TODO warning.
    #[allow(unreachable_code)]
    efi_phase_boot(
        handle,
        system_table,
        todo!("Use EFI boot services to fill boot params"),
    );
}

#[export_name = "efi_handover_entry"]
extern "sysv64" fn efi_handover_entry(
    handle: Handle,
    mut system_table: SystemTable<Boot>,
    boot_params: *mut BootParams,
) -> ! {
    unsafe {
        system_table.boot_services().set_image_handle(handle);
    }
    uefi_services::init(&mut system_table).unwrap();

    efi_phase_boot(handle, system_table, boot_params)
}

fn efi_phase_boot(
    handle: Handle,
    system_table: SystemTable<Boot>,
    boot_params_ptr: *mut BootParams,
) -> ! {
    // Safety: this init function is only called once.
    unsafe { crate::console::init() };

    // Safety: this is the right time to apply relocations.
    unsafe { apply_rela_dyn_relocations() };

    uefi_services::println!("[EFI stub] Relocations applied.");

    uefi_services::println!("[EFI stub] Loading payload.");
    let payload = unsafe { crate::get_payload(&*boot_params_ptr) };
    crate::loader::load_elf(payload);

    uefi_services::println!("[EFI stub] Exiting EFI boot services.");
    let memory_type = {
        let boot_services = system_table.boot_services();
        let Ok(loaded_image) = boot_services.open_protocol_exclusive::<LoadedImage>(handle) else {
            panic!("Failed to open LoadedImage protocol");
        };
        loaded_image.data_type().clone()
    };
    let (system_table, memory_map) = system_table.exit_boot_services(memory_type);

    efi_phase_runtime(system_table, memory_map, boot_params_ptr);
}

fn efi_phase_runtime(
    _system_table: SystemTable<Runtime>,
    memory_map: MemoryMap<'static>,
    boot_params_ptr: *mut BootParams,
) -> ! {
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

    let boot_params = unsafe { &mut *boot_params_ptr };

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
            addr: md.phys_start as u64,
            size: md.page_count as u64 * 4096,
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

    unsafe { super::call_aster_entrypoint(super::ASTER_ENTRY_POINT as u64, boot_params_ptr as u64) }
}
