use core::fmt::Write;
use uefi::{
    data_types::Handle,
    proto::loaded_image::LoadedImage,
    table::{boot::MemoryMap, Boot, Runtime, SystemTable},
};

use linux_boot_params::BootParams;

#[no_mangle]
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

#[no_mangle]
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
    mut system_table: SystemTable<Boot>,
    boot_params: *mut BootParams,
) -> ! {
    // Safety: this init function is only called once.
    unsafe { crate::console::init() };
    
    // Safety: this is a right place to call this function.
    unsafe { crate::x86::relocation::apply_rela_dyn_relocations() };

    uefi_services::println!("[EFI stub] Relocations applied.");
    uefi_services::println!("[EFI stub] Exiting EFI boot services.");
    let memory_type = {
        let boot_services = system_table.boot_services();
        let Ok(loaded_image) = boot_services.open_protocol_exclusive::<LoadedImage>(handle) else {
            panic!("Failed to open LoadedImage protocol");
        };
        loaded_image.data_type().clone()
    };
    let (system_table, memory_map) = system_table.exit_boot_services(memory_type);

    efi_phase_runtime(system_table, memory_map, boot_params);
}

fn efi_phase_runtime(
    mut system_table: SystemTable<Runtime>,
    memory_map: MemoryMap<'static>,
    boot_params: *mut BootParams,
) -> ! {
    unsafe {
        crate::console::print_str("[EFI stub] Entered runtime services.\n");
    }

    #[cfg(feature = "debug_print")]
    unsafe {
        use crate::console::{print_hex, print_str};
        print_str("[EFI stub debug] Memory map:\n");
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

    crate::trojan_entry(boot_params as usize);
}
