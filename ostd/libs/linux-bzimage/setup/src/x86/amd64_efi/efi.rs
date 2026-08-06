// SPDX-License-Identifier: MPL-2.0

extern crate alloc;

use alloc::vec::Vec;
use core::{ffi::CStr, mem::MaybeUninit};

#[cfg(feature = "cvm_guest")]
use align_ext::AlignExt;
use boot::{AllocateType, open_protocol_exclusive};
use linux_boot_params::{AP_BOOT_REGION_SIZE, AP_BOOT_START_PA, BootParams};
#[cfg(feature = "cvm_guest")]
use tdx_guest::{is_tdx_guest_early, unaccepted_memory::EfiUnacceptedMemory};
use uefi::{mem::memory_map::MemoryMap, prelude::*, proto::BootPolicy};
use uefi_raw::table::system::SystemTable;

use super::decoder::decode_payload;
#[cfg(feature = "cvm_guest")]
use super::unaccepted_memory::{
    UnacceptedRange, accept_bitmap_range, allocate_unaccepted_bitmap, find_unaccepted_table,
    install_unaccepted_bitmap,
};
use crate::x86::amd64_efi::alloc::alloc_pages;

#[cfg(feature = "cvm_guest")]
const AP_EXEC_RANGE: (u64, u64) = (AP_BOOT_START_PA, AP_BOOT_START_PA + AP_BOOT_REGION_SIZE);

#[cfg(feature = "cvm_guest")]
type LazyAcceptBootRanges = Vec<(u64, u64)>;

pub(super) const PAGE_SIZE: u64 = 4096;

// SAFETY: The name does not collide with other symbols.
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

    #[cfg(feature = "cvm_guest")]
    {
        let lazy_accept_boot_ranges = efi_phase_boot(boot_params);
        // SAFETY: All previously opened boot service protocols have been closed. At this time, we
        // have no references to the code and data of the boot services.
        unsafe { efi_phase_runtime(boot_params, lazy_accept_boot_ranges) };
    }

    #[cfg(not(feature = "cvm_guest"))]
    {
        efi_phase_boot(boot_params);
        // SAFETY: All previously opened boot service protocols have been closed. At this time, we
        // have no references to the code and data of the boot services.
        unsafe { efi_phase_runtime(boot_params) };
    }
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

fn efi_phase_boot_common(boot_params: &mut BootParams) -> Vec<u8> {
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

    kernel
}

#[cfg(feature = "cvm_guest")]
fn efi_phase_boot(boot_params: &mut BootParams) -> LazyAcceptBootRanges {
    let kernel = efi_phase_boot_common(boot_params);

    compute_kernel_load_ranges(&kernel)
}

#[cfg(not(feature = "cvm_guest"))]
fn efi_phase_boot(boot_params: &mut BootParams) {
    let _kernel = efi_phase_boot_common(boot_params);
}

fn load_cmdline() -> Option<&'static CStr> {
    uefi::println!("[EFI stub] Loading the cmdline");

    let loaded_image =
        open_protocol_exclusive::<uefi::proto::loaded_image::LoadedImage>(boot::image_handle())
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

    let Ok(handle) = boot::locate_device_path::<LoadFile2>(&mut device_path) else {
        uefi::println!("[EFI stub] warning: failed to locate initrd device");
        return None;
    };

    let Ok(mut load_file2) = open_protocol_exclusive::<LoadFile2>(handle) else {
        uefi::println!("[EFI stub] warning: failed to open initrd protocol");
        return None;
    };

    let mut size = 0;
    // SAFETY: The arguments are correctly specified according to the UEFI specification.
    let status = unsafe {
        (load_file2.0.load_file)(
            &mut load_file2.0,
            device_path.as_ffi_ptr().cast(),
            BootPolicy::ExactMatch.into(),
            &mut size,
            core::ptr::null_mut(),
        )
    };
    if status != Status::BUFFER_TOO_SMALL {
        uefi::println!("[EFI stub] warning: failed to get initrd size");
        return None;
    }

    let initrd = alloc_pages(AllocateType::MaxAddress(u32::MAX as u64), size);
    // SAFETY: The arguments are correctly specified according to the UEFI specification.
    let status = unsafe {
        (load_file2.0.load_file)(
            &mut load_file2.0,
            device_path.as_ffi_ptr().cast(),
            BootPolicy::ExactMatch.into(),
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
    use uefi::table::cfg::ConfigTableEntry;

    // Prefer ACPI2 over ACPI.
    for acpi_guid in [ConfigTableEntry::ACPI2_GUID, ConfigTableEntry::ACPI_GUID] {
        if let Some(rsdp_addr) = system::with_config_table(|table| {
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

    let Ok(handle) = boot::get_handle_for_protocol::<GraphicsOutput>() else {
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
                agent: boot::image_handle(),
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

#[cfg(feature = "cvm_guest")]
unsafe fn efi_phase_runtime(
    boot_params: &mut BootParams,
    kernel_load_ranges: LazyAcceptBootRanges,
) -> ! {
    let lazy_accept_boot_info = is_tdx_guest_early().then(prepare_lazy_accept_boot_info);

    if let Some(lazy_accept_boot_info) = lazy_accept_boot_info {
        apply_lazy_accept_boot_acceptance(lazy_accept_boot_info, &kernel_load_ranges, boot_params)
    };

    uefi::println!("[EFI stub] Exiting EFI boot services");
    // SAFETY: The safety is upheld by the caller.
    let memory_map = unsafe { boot::exit_boot_services(Some(boot::MemoryType::LOADER_DATA)) };

    crate::println!(
        "[EFI stub] Processing {} memory map entries",
        memory_map.entries().len()
    );

    populate_e820_from_memory_map(boot_params, &memory_map);

    crate::println!(
        "[EFI stub] Entering the Asterinas entry point at {:p}",
        super::ASTER_ENTRY_POINT,
    );
    // SAFETY:
    // 1. The entry point address is correct and matches the kernel ELF file.
    // 2. The boot parameter pointer is valid and points to the correct boot parameters.
    unsafe { super::call_aster_entrypoint(super::ASTER_ENTRY_POINT, boot_params) }
}

#[cfg(not(feature = "cvm_guest"))]
unsafe fn efi_phase_runtime(boot_params: &mut BootParams) -> ! {
    uefi::println!("[EFI stub] Exiting EFI boot services");
    // SAFETY: The safety is upheld by the caller.
    let memory_map = unsafe { boot::exit_boot_services(Some(boot::MemoryType::LOADER_DATA)) };

    crate::println!(
        "[EFI stub] Processing {} memory map entries",
        memory_map.entries().len()
    );

    populate_e820_from_memory_map(boot_params, &memory_map);

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

fn parse_memory_type(mem_type: boot::MemoryType) -> linux_boot_params::E820Type {
    use linux_boot_params::E820Type;
    use uefi::boot::MemoryType;

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

            crate::println!(
                "[EFI stub] error: UNACCEPTED memory is unsupported outside a TDX guest"
            );
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
fn compute_kernel_load_ranges(kernel: &[u8]) -> Vec<(u64, u64)> {
    let mut kernel_load_ranges: Vec<_> = crate::loader::get_elf_load_ranges(kernel)
        .into_iter()
        .map(|(start, end)| (start.align_down(PAGE_SIZE), end.align_up(PAGE_SIZE)))
        .collect();

    #[cfg(feature = "debug_print")]
    for &(start, end) in &kernel_load_ranges {
        uefi::println!("[EFI stub] Kernel load range: {:#x} - {:#x}", start, end);
    }

    kernel_load_ranges.push(AP_EXEC_RANGE);
    kernel_load_ranges
}

#[cfg(feature = "cvm_guest")]
fn prepare_lazy_accept_boot_info() -> LazyAcceptBootInfo {
    // Collect unaccepted memory ranges from the memory map.
    let pre_exit_memory_map =
        boot::memory_map(boot::MemoryType::LOADER_DATA).unwrap_or_else(|_| {
            panic!("[EFI stub] failed to fetch pre-exit memory map for unaccepted bitmap setup")
        });

    let mut unaccepted_ranges: Vec<_> = pre_exit_memory_map
        .entries()
        .filter(|e| e.ty == boot::MemoryType::UNACCEPTED)
        .map(|e| UnacceptedRange {
            start: e.phys_start,
            end: e.phys_start + e.page_count * PAGE_SIZE,
        })
        .collect();

    let (table, fallback_reserved_range) = if let Some(existing) = find_unaccepted_table() {
        uefi::println!("[EFI stub] Reusing firmware-provided unaccepted memory table");
        (Some(UnacceptedTable::Existing(existing)), None)
    } else if unaccepted_ranges.is_empty() {
        (None, None)
    } else {
        // The OVMF image used by Asterinas exposes unaccepted EFI memory but
        // does not install the Linux unaccepted-memory table.
        uefi::println!(
            "[EFI stub] Firmware did not provide an unaccepted memory table; installing EFI-stub fallback"
        );
        let table = allocate_unaccepted_bitmap(&unaccepted_ranges)
            .unwrap_or_else(|| panic!("[EFI stub] failed to allocate unaccepted bitmap table"));
        install_unaccepted_bitmap(table).unwrap_or_else(|err| {
            panic!(
                "[EFI stub] failed to install unaccepted bitmap table: {:?}",
                err
            )
        });
        let table_addr = u64::try_from(core::ptr::from_mut(table).addr())
            .expect("[EFI stub] fallback unaccepted table address overflowed");
        let table_size = u64::try_from(size_of::<EfiUnacceptedMemory>())
            .expect("[EFI stub] fallback unaccepted header size conversion overflowed")
            .checked_add(table.bitmap_size_bytes())
            .expect("[EFI stub] fallback unaccepted table size overflowed");
        let reserved_start = table_addr.align_down(u64::from(table.unit_size_bytes()));
        let reserved_end = table_addr
            .checked_add(table_size)
            .expect("[EFI stub] fallback unaccepted table end overflowed")
            .align_up(u64::from(table.unit_size_bytes()));

        (
            Some(UnacceptedTable::Fallback(table)),
            Some(UnacceptedRange {
                start: reserved_start,
                end: reserved_end,
            }),
        )
    };

    if let Some(reserved_range) = fallback_reserved_range {
        uefi::println!(
            "[EFI stub] Reserving fallback bitmap-covered range from lazy_accept boot tracking: [{:#x}, {:#x})",
            reserved_range.start,
            reserved_range.end
        );
        unaccepted_ranges = exclude_unaccepted_range(unaccepted_ranges, reserved_range);
    }

    LazyAcceptBootInfo {
        unaccepted_ranges,
        table,
    }
}

#[cfg(feature = "cvm_guest")]
fn exclude_unaccepted_range(
    unaccepted_ranges: Vec<UnacceptedRange>,
    excluded_range: UnacceptedRange,
) -> Vec<UnacceptedRange> {
    let mut filtered_ranges = Vec::with_capacity(unaccepted_ranges.len() + 1);

    for range in unaccepted_ranges {
        if excluded_range.end <= range.start || excluded_range.start >= range.end {
            filtered_ranges.push(range);
            continue;
        }

        if range.start < excluded_range.start {
            filtered_ranges.push(UnacceptedRange {
                start: range.start,
                end: excluded_range.start,
            });
        }

        if excluded_range.end < range.end {
            filtered_ranges.push(UnacceptedRange {
                start: excluded_range.end,
                end: range.end,
            });
        }
    }

    filtered_ranges
}

#[cfg(feature = "cvm_guest")]
fn apply_lazy_accept_boot_acceptance(
    lazy_accept_boot_info: LazyAcceptBootInfo,
    kernel_load_ranges: &[(u64, u64)],
    boot_params: &BootParams,
) {
    let LazyAcceptBootInfo {
        unaccepted_ranges,
        table,
    } = lazy_accept_boot_info;

    if unaccepted_ranges.is_empty() {
        return;
    }

    let Some(table) = table else {
        panic!("[EFI stub] unaccepted memory exists but bitmap table is unavailable");
    };

    let table = match table {
        UnacceptedTable::Existing(table) => table,
        UnacceptedTable::Fallback(table) => {
            for range in &unaccepted_ranges {
                // SAFETY: Range comes from firmware memory map and table is valid/writable.
                if let Err(err) = unsafe { table.register_range(range.start, range.end) } {
                    panic!(
                        "[EFI stub] failed to process unaccepted memory range [{:#x}, {:#x}): {:?}",
                        range.start, range.end, err
                    );
                }
            }
            table
        }
    };

    // The kernel load ranges should be accepted immediately, even if they are covered by the bitmap,
    // to ensure the kernel code and data are accessible when the entry point is called.
    for &(k_start, k_end) in kernel_load_ranges {
        if !accept_bitmap_range(table, k_start, k_end) {
            panic!("[EFI stub] failed to accept kernel mapped unaccepted memory via bitmap");
        }
    }

    // These buffers are read before the kernel can start its SMP-based accept
    // phase, so they must be accepted while still in the EFI stub.
    for (start, end) in boot_critical_ranges(boot_params) {
        if !accept_bitmap_range(table, start, end) {
            panic!("[EFI stub] failed to accept boot-critical memory range");
        }
    }
}

#[cfg(feature = "cvm_guest")]
fn boot_critical_ranges(boot_params: &BootParams) -> Vec<(u64, u64)> {
    let mut ranges = Vec::new();

    push_boot_critical_range(
        &mut ranges,
        u64::try_from(core::ptr::from_ref(boot_params).addr()).unwrap(),
        u64::try_from(size_of::<BootParams>()).unwrap(),
    );

    if boot_params.hdr.cmd_line_ptr != 0 && boot_params.ext_cmd_line_ptr == 0 {
        push_boot_critical_range(
            &mut ranges,
            u64::from(boot_params.hdr.cmd_line_ptr),
            u64::from(boot_params.hdr.cmdline_size),
        );
    }

    if boot_params.hdr.ramdisk_image != 0 && boot_params.ext_ramdisk_image == 0 {
        push_boot_critical_range(
            &mut ranges,
            u64::from(boot_params.hdr.ramdisk_image),
            u64::from(boot_params.hdr.ramdisk_size),
        );
    }

    push_boot_critical_range(&mut ranges, boot_params.acpi_rsdp_addr, PAGE_SIZE);

    if boot_params.screen_info.lfb_base != 0 || boot_params.screen_info.ext_lfb_base != 0 {
        let framebuffer = u64::from(boot_params.screen_info.lfb_base)
            | (u64::from(boot_params.screen_info.ext_lfb_base) << 32);
        let size = u64::from(boot_params.screen_info.lfb_width)
            .checked_mul(u64::from(boot_params.screen_info.lfb_height))
            .and_then(|pixels| pixels.checked_mul(4))
            .unwrap_or(0);
        push_boot_critical_range(&mut ranges, framebuffer, size);
    }

    ranges
}

#[cfg(feature = "cvm_guest")]
fn push_boot_critical_range(ranges: &mut Vec<(u64, u64)>, start: u64, size: u64) {
    if size == 0 {
        return;
    }

    let Some(end) = start.checked_add(size) else {
        return;
    };
    ranges.push((start.align_down(PAGE_SIZE), end.align_up(PAGE_SIZE)));
}

#[cfg(feature = "cvm_guest")]
// Under the lazy_accept policy, defer the full acceptance of ordinary memory until the kernel
// boot phase, when all CPUs can accept disjoint bitmap ranges in parallel.
struct LazyAcceptBootInfo {
    unaccepted_ranges: Vec<UnacceptedRange>,
    table: Option<UnacceptedTable>,
}

#[cfg(feature = "cvm_guest")]
enum UnacceptedTable {
    Existing(&'static mut EfiUnacceptedMemory),
    Fallback(&'static mut EfiUnacceptedMemory),
}
