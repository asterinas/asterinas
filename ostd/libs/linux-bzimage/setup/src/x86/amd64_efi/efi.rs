// SPDX-License-Identifier: MPL-2.0

use core::{ffi::CStr, mem::MaybeUninit};

use boot::{AllocateType, open_protocol_exclusive};
use cfg_if::cfg_if;
use linux_boot_params::BootParams;
use uefi::{boot::exit_boot_services, mem::memory_map::MemoryMap, prelude::*};
use uefi_raw::table::system::SystemTable;

use super::decoder::decode_payload;
use crate::x86::amd64_efi::alloc::alloc_pages;

cfg_if! {
    if #[cfg(feature = "cvm_guest")] {
        extern crate alloc;
        use alloc::vec::Vec;
        use align_ext::AlignExt;
        use tdx_guest::{
            is_tdx_guest_early,
            unaccepted_memory::{BitmapSegmentLock, BitmapSegmentLocks, EfiUnacceptedMemory},
        };
        use super::unaccepted_memory::{
            UnacceptedRange, accept_bitmap_range, allocate_unaccepted_bitmap, find_unaccepted_table,
            install_unaccepted_bitmap,
        };

        /// Safety margin added to the kernel footprint to account for
        /// any additional memory that may be accessed during early boot.
        const KERNEL_FOOTPRINT_SAFETY_MARGIN: u64 = 2 * 1024 * 1024;

        /// AP execution page range required for AP startup.
        /// In the linker script, AP_EXEC_MA = 0x8000.
        const AP_EXEC_RANGE: (u64, u64) = (0x8000, 0x9000);
    }
}

pub(super) const PAGE_SIZE: u64 = 4096;

/// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn main_efi_common64(
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
        let bytes = alloc_pages(AllocateType::AnyPages, size_of::<BootParams>());
        bytes.write_filled(0);
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
    if boot_params.hdr.cmd_line_ptr == 0
        && boot_params.ext_cmd_line_ptr == 0
        && let Some(cmdline) = load_cmdline()
    {
        boot_params.hdr.cmd_line_ptr = cmdline.as_ptr().addr().try_into().unwrap();
        boot_params.ext_cmd_line_ptr = 0;
        boot_params.hdr.cmdline_size = (cmdline.count_bytes() + 1).try_into().unwrap();
    }

    // Load the init ramdisk if it is not loaded.
    if boot_params.hdr.ramdisk_image == 0
        && boot_params.ext_ramdisk_image == 0
        && let Some(initrd) = load_initrd()
    {
        boot_params.hdr.ramdisk_image = initrd.as_ptr().addr().try_into().unwrap();
        boot_params.ext_ramdisk_image = 0;
        boot_params.hdr.ramdisk_size = initrd.len().try_into().unwrap();
        boot_params.ext_ramdisk_size = 0;
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

    // Fill the EFI info in boot params.
    fill_efi_info(boot_params);

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
        uefi::println!("[EFI stub] warning: command line is unavailable");
        return None;
    };

    if !load_options.len().is_multiple_of(2)
        || load_options.iter().skip(1).step_by(2).any(|c| *c != 0)
    {
        uefi::println!("[EFI stub] warning: command line contains non-ASCII characters");
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
        uefi::println!("[EFI stub] warning: failed to locate initrd device");
        return None;
    };

    let Ok(mut load_file2) = uefi::boot::open_protocol_exclusive::<LoadFile2>(handle) else {
        uefi::println!("[EFI stub] warning: failed to open initrd protocol");
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
        uefi::println!("[EFI stub] warning: failed to get initrd size");
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
        uefi::println!("[EFI stub] warning: failed to load initrd");
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
    use uefi::table::cfg::{ACPI_GUID, ACPI2_GUID};

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

    uefi::println!("[EFI stub] warning: failed to find ACPI RSDP address");

    None
}

fn fill_screen_info(screen_info: &mut linux_boot_params::ScreenInfo) {
    use uefi::{
        boot::{OpenProtocolAttributes, OpenProtocolParams, open_protocol},
        proto::console::gop::{GraphicsOutput, PixelFormat},
    };

    let Ok(handle) = uefi::boot::get_handle_for_protocol::<GraphicsOutput>() else {
        uefi::println!("[EFI stub] warning: failed to locate graphics handle");
        return;
    };

    // We don't use `open_protocol_exclusive` here for `GraphicsOutput` because it may disconnect
    // the console.
    //
    // UEFI Specification, 7.3.9 EFI_BOOT_SERVICES.OpenProtocol():
    //   EXCLUSIVE [..] If any drivers have the protocol interface opened with an attribute of
    //   BY_DRIVER, then an attempt will be made to remove them by calling the driver's Stop()
    //   function.
    //
    // SAFETY: No one will change the graphics mode at this point. It is safe to query it through
    // shared access.
    let Ok(mut protocol) = (unsafe {
        open_protocol::<GraphicsOutput>(
            OpenProtocolParams {
                handle,
                agent: uefi::boot::image_handle(),
                controller: None,
            },
            OpenProtocolAttributes::GetProtocol,
        )
    }) else {
        uefi::println!("[EFI stub] warning: failed to open graphics protocol");
        return;
    };

    if !matches!(
        protocol.current_mode_info().pixel_format(),
        PixelFormat::Rgb | PixelFormat::Bgr
    ) {
        uefi::println!(
            "[EFI stub] warning: ignored framebuffer because pixel format is unsupported"
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

    uefi::println!(
        "[EFI stub] Found the framebuffer at {:#x} with {}x{} pixels",
        addr,
        width,
        height
    );
}

fn fill_efi_info(boot_params: &mut BootParams) {
    let Some(system_table) = uefi::table::system_table_raw() else {
        uefi::println!("[EFI stub] warning: EFI system table is unavailable");
        return;
    };

    let systab_addr = u64::try_from(system_table.as_ptr().addr()).unwrap();

    boot_params.efi_info.efi_systab = u32::try_from(systab_addr & 0xFFFF_FFFF).unwrap();
    boot_params.efi_info.efi_systab_hi = u32::try_from(systab_addr >> 32).unwrap();
    boot_params.efi_info.efi_loader_signature = u32::from_le_bytes(*b"EL64");

    #[cfg(feature = "debug_print")]
    {
        let (efi_systab, efi_systab_hi) = (
            boot_params.efi_info.efi_systab,
            boot_params.efi_info.efi_systab_hi,
        );

        uefi::println!(
            "[EFI stub] EFI systab set to {:#x} (lo={:#x}, hi={:#x})",
            systab_addr,
            efi_systab,
            efi_systab_hi
        );
    }
}

unsafe fn efi_phase_runtime(boot_params: &mut BootParams) -> ! {
    #[cfg(feature = "cvm_guest")]
    let unaccepted_info = is_tdx_guest_early()
        .then(|| prepare_lazy_accept_info().unwrap_or_else(|err| panic!("[EFI stub] {}", err)));

    uefi::println!("[EFI stub] Exiting EFI boot services");
    // SAFETY: The safety is upheld by the caller.
    let memory_map = unsafe { exit_boot_services(uefi::table::boot::MemoryType::LOADER_DATA) };

    crate::println!(
        "[EFI stub] Processing {} memory map entries",
        memory_map.entries().len()
    );

    populate_e820_from_memory_map(boot_params, &memory_map);

    #[cfg(feature = "cvm_guest")]
    if let Some((unaccepted_ranges, unaccepted_table, kernel_load_ranges)) =
        unaccepted_info.as_ref()
    {
        apply_lazy_accept_info(unaccepted_ranges, *unaccepted_table, kernel_load_ranges)
    };

    crate::println!(
        "[EFI stub] Entering the Asterinas entry point at {:p}",
        super::ASTER_ENTRY_POINT,
    );
    // SAFETY:
    // 1. The entry point address is correct and matches the kernel ELF file.
    // 2. The boot parameter pointer is valid and points to the correct boot parameters.
    unsafe { super::call_aster_entrypoint(super::ASTER_ENTRY_POINT, boot_params) }
}

fn populate_e820_from_memory_map<M: MemoryMap>(boot_params: &mut BootParams, memory_map: &M) {
    let e820_table = &mut boot_params.e820_table;
    let mut num_entries = 0;

    for entry in memory_map.entries() {
        #[cfg(feature = "debug_print")]
        crate::println!(
            "    [{:#x}] {:#x} (size={:#x}) {{flags={:#x}}}",
            entry.ty.0,
            entry.phys_start,
            entry.page_count,
            entry.att.bits()
        );

        let typ = parse_memory_type(entry.ty);
        if num_entries != 0 {
            let last_entry = &mut e820_table[num_entries - 1];
            let last_typ = last_entry.typ;
            if last_typ == typ && last_entry.addr + last_entry.size == entry.phys_start {
                last_entry.size += entry.page_count * PAGE_SIZE;
                continue;
            }
        }

        if num_entries >= e820_table.len() {
            crate::println!("[EFI stub] warning: number of E820 entries exceeded 128");
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
}

fn parse_memory_type(mem_type: uefi::table::boot::MemoryType) -> linux_boot_params::E820Type {
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
        | MemoryType::CONVENTIONAL => E820Type::Ram,

        // Some memory types have special meanings.
        MemoryType::PERSISTENT_MEMORY => E820Type::Pmem,
        MemoryType::ACPI_RECLAIM => E820Type::Acpi,
        MemoryType::ACPI_NON_VOLATILE => E820Type::Nvs,
        MemoryType::UNUSABLE => E820Type::Unusable,
        MemoryType::UNACCEPTED => {
            #[cfg(feature = "cvm_guest")]
            {
                if is_tdx_guest_early() {
                    return E820Type::Ram;
                }
            }

            crate::println!("[EFI stub] error: cannot classify unknown EFI memory type");
            E820Type::Reserved
        }

        // Other memory types are treated as reserved.
        MemoryType::RESERVED
        | MemoryType::RUNTIME_SERVICES_CODE
        | MemoryType::RUNTIME_SERVICES_DATA
        | MemoryType::MMIO
        | MemoryType::MMIO_PORT_SPACE => E820Type::Reserved,
        _ => E820Type::Reserved,
    }
}

#[cfg(feature = "cvm_guest")]
fn prepare_lazy_accept_info() -> LazyAcceptInfoResult {
    let kernel = decode_payload(crate::x86::payload());
    // 1. Get the original physical segment defined by ELF.
    let raw_kernel_ranges = crate::loader::elf_load_ranges(&kernel);

    // 2. Calculate the physical footprint of the kernel and add a safety margin.
    let mut kernel_load_ranges = Vec::new();
    if let (Some(&(min_start, _)), Some(&(_, max_end))) =
        (raw_kernel_ranges.first(), raw_kernel_ranges.last())
    {
        let footprint_end = (max_end + KERNEL_FOOTPRINT_SAFETY_MARGIN).align_up(PAGE_SIZE);
        kernel_load_ranges.push((min_start, footprint_end));

        uefi::println!(
            "[EFI stub] Kernel footprint calculated: {:#x} - {:#x}",
            min_start,
            footprint_end
        );
    }

    // 3. Add the AP execution page range, required for AP startup.
    kernel_load_ranges.push(AP_EXEC_RANGE);

    // 4. Collect unaccepted memory ranges from the memory map.
    let pre_exit_memory_map = uefi::boot::memory_map(uefi::table::boot::MemoryType::LOADER_DATA)
        .map_err(|_| "failed to fetch pre-exit memory map for unaccepted bitmap setup")?;

    let unaccepted_ranges: Vec<_> = pre_exit_memory_map
        .entries()
        .filter(|e| e.ty == uefi::table::boot::MemoryType::UNACCEPTED)
        .map(|e| UnacceptedRange {
            start: e.phys_start,
            end: e.phys_start + e.page_count * PAGE_SIZE,
        })
        .collect();

    let unaccepted_table = if let Some(existing) = find_unaccepted_table() {
        Some(core::ptr::NonNull::from(existing))
    } else if unaccepted_ranges.is_empty() {
        None
    } else {
        let table = allocate_unaccepted_bitmap(&unaccepted_ranges)
            .unwrap_or_else(|| panic!("[EFI stub] failed to allocate unaccepted bitmap table"));
        install_unaccepted_bitmap(table).unwrap_or_else(|err| {
            panic!(
                "[EFI stub] failed to install unaccepted bitmap table: {:?}",
                err
            )
        });
        Some(core::ptr::NonNull::from(table))
    };

    Ok((unaccepted_ranges, unaccepted_table, kernel_load_ranges))
}

#[cfg(feature = "cvm_guest")]
fn apply_lazy_accept_info(
    unaccepted_ranges: &[UnacceptedRange],
    unaccepted_table: Option<core::ptr::NonNull<EfiUnacceptedMemory>>,
    kernel_load_ranges: &[(u64, u64)],
) {
    if unaccepted_ranges.is_empty() {
        return;
    }

    let Some(unaccepted_table) = unaccepted_table else {
        panic!("[EFI stub] unaccepted memory exists but bitmap table is unavailable");
    };

    // SAFETY: `unaccepted_table` is returned by EFI allocation and remains valid for kernel.
    let table = unsafe { &mut *unaccepted_table.as_ptr() };
    let bitmap_locks = [BitmapSegmentLock::new(false)];
    let locks = BitmapSegmentLocks::new(&bitmap_locks, u64::MAX)
        .unwrap_or_else(|err| panic!("[EFI stub] failed to create bitmap lock table: {:?}", err));

    for range in unaccepted_ranges {
        // SAFETY: Range comes from firmware memory map and table is valid/writable.
        if let Err(err) = unsafe { table.register_range(range.start, range.end, &locks) } {
            panic!(
                "[EFI stub] failed to process unaccepted memory range [{:#x}, {:#x}): {:?}",
                range.start, range.end, err
            );
        }
    }

    // The kernel load ranges should be accepted immediately, even if they are covered by the bitmap,
    // to ensure the kernel code and data are accessible when the entry point is called.
    for &(k_start, k_end) in kernel_load_ranges {
        if !accept_bitmap_range(table, k_start, k_end, &locks) {
            panic!("[EFI stub] failed to accept kernel mapped unaccepted memory via bitmap");
        }
    }
}

#[cfg(feature = "cvm_guest")]
type LazyAcceptInfo = (
    Vec<UnacceptedRange>,
    Option<core::ptr::NonNull<EfiUnacceptedMemory>>,
    Vec<(u64, u64)>,
);

#[cfg(feature = "cvm_guest")]
type LazyAcceptInfoResult = Result<LazyAcceptInfo, &'static str>;
