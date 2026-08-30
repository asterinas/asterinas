// SPDX-License-Identifier: MPL-2.0

//! The Linux 64-bit Boot Protocol supporting module.
//!

use linux_boot_params::{BootParams, E820Type, LINUX_BOOT_HEADER_MAGIC};

use super::ToEarlyBootInfo;
#[cfg(feature = "cvm_guest")]
use crate::arch::init_cvm_guest;
use crate::{
    arch::if_tdx_enabled,
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
    mm::kspace::paddr_to_vaddr,
};

fn is_efi_boot(boot_params: &BootParams) -> bool {
    const EFI32_LOADER_SIGNATURE: u32 = u32::from_le_bytes(*b"EL32");
    const EFI64_LOADER_SIGNATURE: u32 = u32::from_le_bytes(*b"EL64");

    let efi_info = boot_params.efi_info;
    matches!(
        efi_info.efi_loader_signature,
        EFI32_LOADER_SIGNATURE | EFI64_LOADER_SIGNATURE
    )
}

impl From<E820Type> for MemoryRegionType {
    fn from(value: E820Type) -> Self {
        match value {
            E820Type::Ram => Self::Usable,
            E820Type::Reserved => Self::Reserved,
            E820Type::Acpi => Self::Reclaimable,
            E820Type::Nvs => Self::NonVolatileSleep,
            E820Type::Unusable => Self::BadMemory,
            // All other memory regions are reserved.
            // FIXME: Using Rust enum in this way can be unsound if the bootloader passes an
            // unknown memory type to the kernel (e.g., due to a newer protocol version).
            _ => Self::Reserved,
        }
    }
}

impl ToEarlyBootInfo for BootParams {
    fn bootloader_name(&self) -> &'static str {
        // The bootloaders have assigned IDs in Linux, see
        // https://www.kernel.org/doc/Documentation/x86/boot.txt
        // for details.
        match self.hdr.type_of_loader {
            0x0 => "LILO", // (0x00 reserved for pre-2.00 bootloader)
            0x1 => "Loadlin",
            0x2 => "bootsect-loader", // (0x20, all other values reserved)
            0x3 => "Syslinux",
            0x4 => "Etherboot/gPXE/iPXE",
            0x5 => "ELILO",
            0x7 => "GRUB",
            0x8 => "U-Boot",
            0x9 => "Xen",
            0xA => "Gujin",
            0xB => "Qemu",
            0xC => "Arcturus Networks uCbootloader",
            0xD => "kexec-tools",
            0xE => "Extended loader",
            0xF => "Special", // (0xFF = undefined)
            0x10 => "Reserved",
            0x11 => "Minimal Linux Bootloader <http://sebastian-plotz.blogspot.de>",
            0x12 => "OVMF UEFI virtualization stack",
            _ => "Unknown Linux Loader",
        }
    }

    fn kernel_commandline(&self) -> Option<&'static str> {
        if self.ext_cmd_line_ptr != 0 {
            // TODO: We can support the above 4GiB command line after setting up
            // linear mappings. By far, we cannot log the error because the serial is
            // not up. Proceed as if there was no command line.
            return None;
        }

        if self.hdr.cmd_line_ptr == 0 || self.hdr.cmdline_size == 0 {
            return None;
        }

        let cmdline_ptr = paddr_to_vaddr(self.hdr.cmd_line_ptr as usize);
        let cmdline_len = self.hdr.cmdline_size as usize;
        // SAFETY:
        // 1. The command line is safe to read because of the contract with the loader.
        // 2. We reserve the command-line region in `finish_memory_regions`, so it will live as an
        //    immutable reference for `'static`.
        let cmdline = unsafe { core::slice::from_raw_parts(cmdline_ptr as *const u8, cmdline_len) };

        // Now, unfortunately, there are silent errors because the serial is not up.
        core::ffi::CStr::from_bytes_until_nul(cmdline)
            .ok()?
            .to_str()
            .ok()
    }

    fn initramfs(&self) -> Option<&'static [u8]> {
        if self.ext_ramdisk_image != 0 || self.ext_ramdisk_size != 0 {
            // See the explanation in `kernel_commandline`.
            return None;
        }

        if self.hdr.ramdisk_image == 0 || self.hdr.ramdisk_size == 0 {
            return None;
        }

        let initramfs_ptr = paddr_to_vaddr(self.hdr.ramdisk_image as usize);
        let initramfs_len = self.hdr.ramdisk_size as usize;
        // SAFETY:
        // 1. The initramfs is safe to read because of the contract with the loader.
        // 2. We reserve the initramfs region in `memory_regions`, so it will live as an immutable
        //    reference for `'static`.
        let initramfs =
            unsafe { core::slice::from_raw_parts(initramfs_ptr as *const u8, initramfs_len) };

        Some(initramfs)
    }

    fn acpi_arg(&self) -> BootloaderAcpiArg {
        let rsdp = self.acpi_rsdp_addr;

        if rsdp == 0 {
            if is_efi_boot(self) {
                BootloaderAcpiArg::NotProvided
            } else {
                BootloaderAcpiArg::ScanBios
            }
        } else {
            BootloaderAcpiArg::Rsdp(rsdp.try_into().expect("RSDP address overflowed!"))
        }
    }

    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg> {
        let screen_info = self.screen_info;

        let address = screen_info.lfb_base as usize | ((screen_info.ext_lfb_base as usize) << 32);
        if address == 0 {
            return None;
        }

        Some(BootloaderFramebufferArg {
            address,
            width: screen_info.lfb_width as usize,
            height: screen_info.lfb_height as usize,
            bpp: screen_info.lfb_depth as usize,
        })
    }

    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray {
        let mut regions = MemoryRegionArray::new();

        // Add regions from E820.
        let num_entries = self.e820_entries as usize;
        for e820_entry in &self.e820_table[0..num_entries] {
            regions
                .push(MemoryRegion::new(
                    e820_entry.addr.try_into().unwrap(),
                    e820_entry.size.try_into().unwrap(),
                    e820_entry.typ.into(),
                ))
                .unwrap();
        }

        // FIXME: Early versions of TDVF did not correctly report the location of AP's page tables as
        // EfiACPIMemoryNVS. We need to manually reserve this memory region to prevent them from being
        // corrupted. TDVF has now been upstreamed to OVMF, and this issue has been fixed in OVMF
        // stable-202411 or later. See the commit for details:
        // <https://github.com/tianocore/edk2/commit/383f729ac096b8deb279933fce86e83a5f7f5ec7>.
        if_tdx_enabled!({
            // The definition of these constants can be found in:
            // <https://github.com/tianocore/edk2/blob/a7ab45ace25c4b987994158687d04de07ed20a96/OvmfPkg/IntelTdx/IntelTdxX64.fdf#L64-L71>
            // <https://github.com/tianocore/edk2/blob/a7ab45ace25c4b987994158687d04de07ed20a96/OvmfPkg/Include/Fdf/OvmfPkgDefines.fdf.inc#L106>
            regions
                .push(MemoryRegion::new(
                    // PcdOvmfSecPageTablesBase = $(MEMFD_BASE_ADDRESS) + 0x000000 = 0x800000
                    0x800000,
                    // PcdOvmfSecPageTablesSize = 0x006000
                    0x006000,
                    // EfiACPIMemoryNVS
                    MemoryRegionType::NonVolatileSleep,
                ))
                .unwrap();
        });

        super::finish_memory_regions(regions, framebuffer_arg, initramfs, kernel_cmdline)
    }
}

/// The entry point of the Rust code portion of Asterinas (with Linux boot parameters).
///
/// # Safety
///
/// - This function must be called only once at a proper timing in the BSP's boot assembly code.
/// - The caller must follow C calling conventions and put the right arguments in registers.
/// - If this function is called, entry points of other boot protocols must never be called.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn __linux_boot(params_ptr: *const BootParams) -> ! {
    let params = unsafe { &*params_ptr };
    assert_eq!({ params.hdr.header }, LINUX_BOOT_HEADER_MAGIC);

    use crate::boot::{EARLY_INFO, start_kernel};

    #[cfg(feature = "cvm_guest")]
    init_cvm_guest();

    EARLY_INFO.call_once(|| params.to_early_boot_info());

    // SAFETY: The safety is guaranteed by the safety preconditions and the fact that we call it
    // once after setting up necessary resources.
    unsafe { start_kernel() };
}
