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
        BootloaderAcpiArg, BootloaderFramebufferArg, FramebufferRgbLayout,
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
        let screen = self.screen_info;
        let address = screen.lfb_base as usize | ((screen.ext_lfb_base as usize) << 32);
        let layout = (screen.red_size != 0 || screen.green_size != 0 || screen.blue_size != 0)
            .then(|| {
                FramebufferRgbLayout::new(
                    (screen.red_pos, screen.red_size),
                    (screen.green_pos, screen.green_size),
                    (screen.blue_pos, screen.blue_size),
                )
            });
        // Older EFI stubs did not fill in the scanline length.
        let pitch_bytes = if screen.lfb_linelength == 0 {
            usize::from(screen.lfb_width).checked_mul(usize::from(screen.lfb_depth).div_ceil(8))?
        } else {
            usize::from(screen.lfb_linelength)
        };
        let framebuffer = BootloaderFramebufferArg::new(
            address,
            usize::from(screen.lfb_width),
            usize::from(screen.lfb_height),
            usize::from(screen.lfb_depth),
            pitch_bytes,
            layout,
        )?;
        Some(framebuffer.with_physical_size_mm(self.edid_info.physical_size_mm()))
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

#[cfg(ktest)]
mod test {
    use core::{mem::MaybeUninit, ptr};

    use linux_boot_params::{BootE820Entry, EdidInfo};

    use super::*;
    use crate::prelude::ktest;

    #[ktest]
    fn linux_framebuffer_preserves_pitch_and_channels() {
        let mut params = boot_params_with_framebuffer();
        for (red_pos, blue_pos) in [(0, 16), (16, 0)] {
            params.screen_info.red_pos = red_pos;
            params.screen_info.blue_pos = blue_pos;

            let fb = params.framebuffer_arg().unwrap();
            assert_eq!(fb.physical_range(), 0x1_0000_1000..0x1_0000_1020);
            assert_eq!((fb.width(), fb.height(), fb.bits_per_pixel()), (3, 2, 32));
            assert_eq!(fb.pitch_bytes(), 16);
            let layout = fb.rgb_layout().unwrap();
            assert_eq!(layout.red(), (red_pos, 8));
            assert_eq!(layout.green(), (8, 8));
            assert_eq!(layout.blue(), (blue_pos, 8));
        }

        params.screen_info.lfb_linelength = 11;
        assert!(params.framebuffer_arg().is_none());
    }

    #[ktest]
    fn linux_framebuffer_accepts_legacy_metadata() {
        let mut params = boot_params_with_framebuffer();
        params.screen_info.lfb_linelength = 0;
        params.screen_info.lfb_depth = 24;
        params.screen_info.red_size = 0;
        params.screen_info.green_size = 0;
        params.screen_info.blue_size = 0;

        let fb = params.framebuffer_arg().unwrap();
        assert_eq!(fb.pitch_bytes(), 9);
        assert_eq!(fb.physical_range(), 0x1_0000_1000..0x1_0000_1012);
        assert!(fb.rgb_layout().is_none());
    }

    #[ktest]
    fn linux_framebuffer_preserves_edid_physical_dimensions() {
        let mut params = boot_params_with_framebuffer();
        params.edid_info = display_edid(52, 29);

        let fb = params.framebuffer_arg().unwrap();
        assert_eq!(fb.physical_size_mm(), Some((520, 290)));
        assert_eq!((fb.width(), fb.height()), (3, 2));
    }

    #[ktest]
    fn linux_framebuffer_preserves_unknown_physical_dimensions() {
        let mut params = boot_params_with_framebuffer();
        assert_eq!(params.framebuffer_arg().unwrap().physical_size_mm(), None);

        // EDID 1.4 may describe only an aspect ratio, not physical dimensions.
        for (width_cm, height_cm) in [(0, 0), (52, 0), (0, 29)] {
            params.edid_info = display_edid(width_cm, height_cm);
            assert_eq!(params.framebuffer_arg().unwrap().physical_size_mm(), None);
        }
    }

    fn display_edid(width_cm: u8, height_cm: u8) -> EdidInfo {
        let mut bytes = [0; linux_boot_params::EDID_BASE_BLOCK_SIZE];
        bytes[..8].copy_from_slice(&[0x00, 0xff, 0xff, 0xff, 0xff, 0xff, 0xff, 0x00]);
        bytes[18] = 1;
        bytes[19] = 4;
        bytes[21] = width_cm;
        bytes[22] = height_cm;
        bytes[linux_boot_params::EDID_BASE_BLOCK_SIZE - 1] =
            0u8.wrapping_sub(bytes.iter().fold(0u8, |sum, byte| sum.wrapping_add(*byte)));
        EdidInfo::from_bytes(&bytes).unwrap()
    }

    fn boot_params_with_framebuffer() -> BootParams {
        let mut params = MaybeUninit::<BootParams>::zeroed();
        // SAFETY:
        // 1. All fields except the E820 entry types admit zero.
        // 2. Every E820 entry is initialized with a valid enum discriminant
        //    before the `BootParams` value is created.
        let mut params = unsafe {
            ptr::addr_of_mut!((*params.as_mut_ptr()).e820_table).write(core::array::from_fn(
                |_| BootE820Entry {
                    addr: 0,
                    size: 0,
                    typ: E820Type::Reserved,
                },
            ));
            params.assume_init()
        };
        params.screen_info.lfb_base = 0x1000;
        params.screen_info.ext_lfb_base = 1;
        params.screen_info.lfb_width = 3;
        params.screen_info.lfb_height = 2;
        params.screen_info.lfb_depth = 32;
        params.screen_info.lfb_linelength = 16;
        params.screen_info.red_pos = 16;
        params.screen_info.red_size = 8;
        params.screen_info.green_pos = 8;
        params.screen_info.green_size = 8;
        params.screen_info.blue_pos = 0;
        params.screen_info.blue_size = 8;
        params
    }
}
