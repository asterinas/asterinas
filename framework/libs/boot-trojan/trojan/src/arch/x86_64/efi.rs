use core::fmt::Write;
use uefi::{
    data_types::Handle,
    proto::loaded_image::LoadedImage,
    table::{Boot, SystemTable},
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
    efi_entry(
        handle,
        system_table,
        todo!("Use EFI services to fill boot params"),
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

    efi_entry(handle, system_table, boot_params)
}

fn efi_entry(
    handle: Handle,
    mut system_table: SystemTable<Boot>,
    boot_params: *mut BootParams,
) -> ! {
    system_table
        .stdout()
        .write_str("[EFI stub] Exiting EFI boot services.\n")
        .unwrap();
    let memory_type = {
        let boot_services = system_table.boot_services();
        let Ok(loaded_image) = boot_services.open_protocol_exclusive::<LoadedImage>(handle) else {
            panic!("Failed to open LoadedImage protocol");
        };
        loaded_image.data_type().clone()
    };
    let _ = system_table.exit_boot_services(memory_type);

    let loaded_base = {
        extern "C" {
            fn start_of_setup32();
        }
        start_of_setup32 as usize
    };

    crate::trojan_entry(loaded_base, boot_params as usize);
}
