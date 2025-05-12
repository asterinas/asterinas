// SPDX-License-Identifier: MPL-2.0

use core::{ffi::CStr, mem::MaybeUninit};

use boot::{open_protocol_exclusive, AllocateType};
use linux_boot_params::BootParams;
use uefi::{boot::exit_boot_services, mem::memory_map::MemoryMap, prelude::*};
use uefi_raw::table::system::SystemTable;

use super::decoder::decode_payload;
use crate::x86::amd64_efi::alloc::alloc_pages;

pub(super) const PAGE_SIZE: u64 = 4096;

#[export_name = "main_efi_common64"]
extern "sysv64" fn main_efi_common64(
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

    let boot_params = if boot_params_ptr.is_null() {
        allocate_boot_params()
    } else {
        // SAFETY: We get boot parameters from the boot loader, so by contract the pointer is valid and
        // the underlying memory is initialized. We are an exclusive owner of the memory region, so we
        // can create a mutable reference of the plain-old-data type.
        unsafe { &mut *boot_params_ptr }
    };

    efi_phase_boot(boot_params);

    // SAFETY: All previously opened boot service protocols have been closed. At this time, we have
    // no references to the code and data of the boot services.
    unsafe { efi_phase_runtime(boot_params) };
}

fn allocate_boot_params() -> &'static mut BootParams {
    let boot_params = {
        let bytes = alloc_pages(AllocateType::AnyPages, core::mem::size_of::<BootParams>());
        MaybeUninit::fill(bytes, 0);
        // SAFETY: Zero initialization gives a valid representation for `BootParams`.
        unsafe { &mut *bytes.as_mut_ptr().cast::<BootParams>() }
    };

    boot_params.hdr.header = linux_boot_params::LINUX_BOOT_HEADER_MAGIC;

    boot_params
}

fn efi_phase_boot(boot_params: &mut BootParams) {
    uefi::println!(
        "[EFI stub] Loaded with offset {:#x}",
        crate::x86::image_load_offset(),
    );

    // Load the command line if it is not loaded.
    if boot_params.hdr.cmd_line_ptr == 0 && boot_params.ext_cmd_line_ptr == 0 {
        if let Some(cmdline) = load_cmdline() {
            boot_params.hdr.cmd_line_ptr = cmdline.as_ptr().addr().try_into().unwrap();
            boot_params.ext_cmd_line_ptr = 0;
            boot_params.hdr.cmdline_size = (cmdline.count_bytes() + 1).try_into().unwrap();
        }
    }

    // Load the init ramdisk if it is not loaded.
    if boot_params.hdr.ramdisk_image == 0 && boot_params.ext_ramdisk_image == 0 {
        if let Some(initrd) = load_initrd() {
            boot_params.hdr.ramdisk_image = initrd.as_ptr().addr().try_into().unwrap();
            boot_params.ext_ramdisk_image = 0;
            boot_params.hdr.ramdisk_size = initrd.len().try_into().unwrap();
            boot_params.ext_ramdisk_size = 0;
        }
    }

    // Fill the boot params with the RSDP address if it is not provided.
    if boot_params.acpi_rsdp_addr == 0 {
        boot_params.acpi_rsdp_addr =
            find_rsdp_addr().expect("ACPI RSDP address is not available") as usize as u64;
    }

    // Fill the boot params with the screen info if it is not provided.
    if boot_params.screen_info.lfb_base == 0 && boot_params.screen_info.ext_lfb_base == 0 {
        fill_screen_info(&mut boot_params.screen_info);
    }

    // Decode the payload and load it as an ELF file.
    uefi::println!("[EFI stub] Decoding the kernel payload");
    let kernel = decode_payload(crate::x86::payload());
    uefi::println!("[EFI stub] Loading the payload as an ELF file");
    crate::loader::load_elf(&kernel);
}

fn load_cmdline() -> Option<&'static CStr> {
    uefi::println!("[EFI stub] Loading the cmdline");

    let loaded_image = open_protocol_exclusive::<uefi::proto::loaded_image::LoadedImage>(
        uefi::boot::image_handle(),
    )
    .unwrap();

    let Some(load_options) = loaded_image.load_options_as_bytes() else {
        uefi::println!("[EFI stub] Warning: No cmdline is available!");
        return None;
    };

    if load_options.len() % 2 != 0 || load_options.iter().skip(1).step_by(2).any(|c| *c != 0) {
        uefi::println!("[EFI stub] Warning: The cmdline contains non-ASCII characters!");
        return None;
    }

    // The load options are a `Char16` sequence. We should convert it to a `Char8` sequence.
    let cmdline_bytes = alloc_pages(
        AllocateType::MaxAddress(u32::MAX as u64),
        load_options.len() / 2 + 1,
    );
    for i in 0..load_options.len() / 2 {
        cmdline_bytes[i].write(load_options[i * 2]);
    }
    cmdline_bytes[load_options.len() / 2].write(0);

    // SAFETY: We've initialized all the bytes above.
    let cmdline_str =
        CStr::from_bytes_until_nul(unsafe { cmdline_bytes.assume_init_ref() }).unwrap();

    uefi::println!("[EFI stub] Loaded the cmdline: {:?}", cmdline_str);

    Some(cmdline_str)
}

// Linux loads the initrd either using a special protocol `LINUX_EFI_INITRD_MEDIA_GUID` or using
// the file path specified on the command line (e.g., `initrd=/initrd.img`). We now only support
// the former approach, as it is more "modern" and easier to implement. Note that this approach
// requires the boot loader (e.g., GRUB, systemd-boot) to implement the protocol, while the latter
// approach does not.
fn load_initrd() -> Option<&'static [u8]> {
    uefi::println!("[EFI stub] Loading the initrd");

    // Note that we should switch to `uefi::proto::media::load_file::LoadFile2` once it provides a
    // more ergonomic API. Its current API requires `alloc` and cannot load files on pages (i.e.,
    // ensure that the initrd is aligned to the page size).
    #[repr(transparent)]
    // SAFETY: The protocol GUID matches the protocol itself.
    #[uefi::proto::unsafe_protocol(uefi_raw::protocol::media::LoadFile2Protocol::GUID)]
    struct LoadFile2(uefi_raw::protocol::media::LoadFile2Protocol);

    let mut device_path_buf = [MaybeUninit::uninit(); 20 /* vendor */ + 4 /* end */];
    let mut device_path = {
        use uefi::proto::device_path::build;
        build::DevicePathBuilder::with_buf(&mut device_path_buf)
            .push(&build::media::Vendor {
                // LINUX_EFI_INITRD_MEDIA_GUID
                vendor_guid: uefi::guid!("5568e427-68fc-4f3d-ac74-ca555231cc68"),
                vendor_defined_data: &[],
            })
            .unwrap()
            .finalize()
            .unwrap()
    };

    let Ok(handle) = uefi::boot::locate_device_path::<LoadFile2>(&mut device_path) else {
        uefi::println!("[EFI stub] Warning: Failed to locate the initrd device!");
        return None;
    };

    let Ok(mut load_file2) = uefi::boot::open_protocol_exclusive::<LoadFile2>(handle) else {
        uefi::println!("[EFI stub] Warning: Failed to open the initrd protocol!");
        return None;
    };

    let mut size = 0;
    // SAFETY: The arguments are correctly specified according to the UEFI specification.
    let status = unsafe {
        (load_file2.0.load_file)(
            &mut load_file2.0,
            device_path.as_ffi_ptr().cast(),
            false, /* boot_policy */
            &mut size,
            core::ptr::null_mut(),
        )
    };
    if status != uefi::Status::BUFFER_TOO_SMALL {
        uefi::println!("[EFI stub] Warning: Failed to get the initrd size!");
        return None;
    }

    let initrd = alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), size);
    // SAFETY: The arguments are correctly specified according to the UEFI specification.
    let status = unsafe {
        (load_file2.0.load_file)(
            &mut load_file2.0,
            device_path.as_ffi_ptr().cast(),
            false, /* boot_policy */
            &mut size,
            initrd.as_mut_ptr().cast(),
        )
    };
    if status.is_error() {
        uefi::println!("[EFI stub] Warning: Failed to load the initrd!");
        return None;
    }
    assert_eq!(
        size,
        initrd.len(),
        "the initrd size has changed between two EFI calls"
    );

    uefi::println!(
        "[EFI stub] Loaded the initrd: addr={:#x}, size={:#x}",
        initrd.as_ptr().addr(),
        initrd.len()
    );

    // SAFETY: We've initialized all the bytes in `load_file`.
    Some(unsafe { initrd.assume_init_ref() })
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
            uefi::println!("[EFI stub] Found the ACPI RSDP at {:p}", rsdp_addr);
            return Some(rsdp_addr);
        }
    }

    uefi::println!("[EFI stub] Warning: Failed to find the ACPI RSDP address!");

    None
}

fn extract_color_mask_info(mask: u32) -> (u8, u8) {
    if mask == 0 {
        // SAFETY: Handle the zero case while initializing the graphic devices.
        return (0, 0);
    }

    let pos = mask.trailing_zeros() as u8;

    let size = (mask >> pos).count_ones() as u8;

    (size, pos)
}

fn fill_screen_info(screen_info: &mut linux_boot_params::ScreenInfo) {
    use uefi::proto::console::gop::{GraphicsOutput, PixelFormat};

    let Ok(handle) = uefi::boot::get_handle_for_protocol::<GraphicsOutput>() else {
        uefi::println!("[EFI stub] Warning: Failed to locate the graphics handle!");
        return;
    };

    let Ok(mut protocol) = open_protocol_exclusive::<GraphicsOutput>(handle) else {
        uefi::println!("[EFI stub] Warning: Failed to open the graphics protocol!");
        return;
    };

    if !matches!(
        protocol.current_mode_info().pixel_format(),
        PixelFormat::Rgb | PixelFormat::Bgr
    ) {
        uefi::println!(
            "[EFI stub] Warning: Ignored the framebuffer as the pixel format is not supported!"
        );
        return;
    }

    let addr = protocol.frame_buffer().as_mut_ptr().addr();
    let (width, height) = protocol.current_mode_info().resolution();

    // TODO: We are only filling in fields that will be accessed later in the kernel. We should
    // fill in other important information such as the pixel format.
    screen_info.lfb_base = addr as u32;
    screen_info.ext_lfb_base = (addr >> 32) as u32;
    screen_info.lfb_width = width.try_into().unwrap();
    screen_info.lfb_height = height.try_into().unwrap();
    screen_info.lfb_depth = 32; // We've checked the pixel format above.

    if let Some(bitmask) = protocol.current_mode_info().pixel_bitmask() {
        (screen_info.red_size, screen_info.red_pos) = extract_color_mask_info(bitmask.red);
        (screen_info.green_size, screen_info.green_pos) = extract_color_mask_info(bitmask.green);
        (screen_info.blue_size, screen_info.blue_pos) = extract_color_mask_info(bitmask.blue);
        (screen_info.rsvd_size, screen_info.rsvd_pos) = extract_color_mask_info(bitmask.reserved);
    } else {
        // The pixel format is not parsed, use the default values.
        screen_info.red_size = 8;
        screen_info.red_pos = 16;
        screen_info.green_size = 8;
        screen_info.green_pos = 8;
        screen_info.blue_size = 8;
        screen_info.blue_pos = 0;
        screen_info.rsvd_size = 8;
        screen_info.rsvd_pos = 24;
    }

    uefi::println!(
        "[EFI stub] Found the framebuffer at {:#x} with {}x{} pixels",
        addr,
        width,
        height
    );
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
