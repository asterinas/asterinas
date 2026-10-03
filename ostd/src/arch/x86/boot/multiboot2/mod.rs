// SPDX-License-Identifier: MPL-2.0

use core::arch::global_asm;

use multiboot2::{BootInformation, BootInformationHeader, MemoryAreaType};

use super::ToEarlyBootInfo;
use crate::{
    boot::{
        BootloaderAcpiArg, BootloaderFramebufferArg, FramebufferRgbLayout,
        memory_region::{MemoryRegion, MemoryRegionArray, MemoryRegionType},
    },
    mm::{Paddr, kspace::paddr_to_vaddr},
};

global_asm!(include_str!("header.S"));

unsafe fn make_str_vaddr_static(str: &str) -> &'static str {
    let vaddr = paddr_to_vaddr(str.as_ptr() as Paddr);

    // SAFETY: The safety is upheld by the caller.
    let bytes = unsafe { core::slice::from_raw_parts(vaddr as *const u8, str.len()) };

    core::str::from_utf8(bytes).unwrap()
}

fn is_efi_boot(mb2_info: &BootInformation) -> bool {
    mb2_info.efi_sdt32_tag().is_some()
        || mb2_info.efi_sdt64_tag().is_some()
        || mb2_info.efi_memory_map_tag().is_some()
        || mb2_info.efi_bs_not_exited_tag().is_some()
        || mb2_info.efi_ih32_tag().is_some()
        || mb2_info.efi_ih64_tag().is_some()
}

impl From<MemoryAreaType> for MemoryRegionType {
    fn from(value: MemoryAreaType) -> Self {
        match value {
            MemoryAreaType::Available => Self::Usable,
            MemoryAreaType::Reserved => Self::Reserved,
            MemoryAreaType::AcpiAvailable => Self::Reclaimable,
            MemoryAreaType::ReservedHibernate => Self::NonVolatileSleep,
            MemoryAreaType::Defective => Self::BadMemory,
            MemoryAreaType::Custom(_) => Self::Reserved,
        }
    }
}

impl ToEarlyBootInfo for BootInformation<'_> {
    fn bootloader_name(&self) -> &'static str {
        let Some(name) = self.boot_loader_name_tag().and_then(|tag| tag.name().ok()) else {
            return "Unknown Multiboot2 Loader";
        };

        // SAFETY:
        // 1. The bootloader name is safe to read because of the contract with the loader.
        // 2. We reserve the bootloader-name region in `memory_regions`, so it will live as an
        //    immutable reference for `'static`.
        unsafe { make_str_vaddr_static(name) }
    }

    fn kernel_commandline(&self) -> Option<&'static str> {
        let cmdline = self.command_line_tag()?.cmdline().ok()?;

        // SAFETY:
        // 1. The command line is safe to read because of the contract with the loader.
        // 2. We reserve the command-line region in `finish_memory_regions`, so it will live as an
        //    immutable reference for `'static`.
        Some(unsafe { make_str_vaddr_static(cmdline) })
    }

    fn initramfs(&self) -> Option<&'static [u8]> {
        let module_tag = self.module_tags().next()?;

        let initramfs_ptr = paddr_to_vaddr(module_tag.start_address() as usize);
        let initramfs_len = module_tag.module_size() as usize;
        // SAFETY:
        // 1. The initramfs is safe to read because of the contract with the loader.
        // 2. We reserve the initramfs region in `memory_regions`, so it will live as an immutable
        //    reference for `'static`.
        let initramfs =
            unsafe { core::slice::from_raw_parts(initramfs_ptr as *const u8, initramfs_len) };

        Some(initramfs)
    }

    fn acpi_arg(&self) -> BootloaderAcpiArg {
        if let Some(v2_tag) = self.rsdp_v2_tag() {
            // Check for RSDP v2
            BootloaderAcpiArg::Xsdt(v2_tag.xsdt_address())
        } else if let Some(v1_tag) = self.rsdp_v1_tag() {
            // Fall back to RSDP v1
            BootloaderAcpiArg::Rsdt(v1_tag.rsdt_address())
        } else if is_efi_boot(self) {
            BootloaderAcpiArg::NotProvided
        } else {
            BootloaderAcpiArg::ScanBios
        }
    }

    fn framebuffer_arg(&self) -> Option<BootloaderFramebufferArg> {
        let fb = self.framebuffer_tag()?.ok()?;
        let multiboot2::FramebufferType::RGB { red, green, blue } = fb.buffer_type().ok()? else {
            return None;
        };
        let layout = FramebufferRgbLayout::new(
            (red.position, red.size),
            (green.position, green.size),
            (blue.position, blue.size),
        );
        BootloaderFramebufferArg::new(
            usize::try_from(fb.address()).ok()?,
            fb.width() as usize,
            fb.height() as usize,
            usize::from(fb.bpp()),
            fb.pitch() as usize,
            Some(layout),
        )
    }

    fn memory_regions(
        &self,
        initramfs: Option<&'static [u8]>,
        kernel_cmdline: Option<&'static str>,
        framebuffer_arg: Option<BootloaderFramebufferArg>,
    ) -> MemoryRegionArray {
        let mut regions = MemoryRegionArray::new();

        // Add the regions returned by Grub.
        let memory_regions_tag = self
            .memory_map_tag()
            .expect("No memory regions are found in the Multiboot2 header!");
        for region in memory_regions_tag.memory_areas() {
            let start = region.start_address();
            let end = region.end_address();
            let area_typ: MemoryRegionType = MemoryAreaType::from(region.typ()).into();
            regions
                .push(MemoryRegion::new(
                    start.try_into().unwrap(),
                    (end - start).try_into().unwrap(),
                    area_typ,
                ))
                .unwrap();
        }

        // Add the boot loader name region since Grub does not specify it.
        if let Some(name) = self.boot_loader_name_tag().and_then(|tag| tag.name().ok()) {
            regions
                .push(MemoryRegion::new(
                    name.as_ptr() as usize,
                    name.len(),
                    MemoryRegionType::Reclaimable,
                ))
                .unwrap();
        }

        super::finish_memory_regions(regions, framebuffer_arg, initramfs, kernel_cmdline)
    }
}

/// The entry point of the Rust code portion of Asterinas (with multiboot2 parameters).
///
/// # Safety
///
/// - This function must be called only once at a proper timing in the BSP's boot assembly code.
/// - The caller must follow C calling conventions and put the right arguments in registers.
/// - If this function is called, entry points of other boot protocols must never be called.
// SAFETY: The name does not collide with other symbols.
#[unsafe(no_mangle)]
unsafe extern "sysv64" fn __multiboot2_entry(boot_magic: u32, boot_params: u64) -> ! {
    assert_eq!(boot_magic, multiboot2::MAGIC);
    let mb2_info =
        unsafe { BootInformation::load(boot_params as *const BootInformationHeader).unwrap() };

    use crate::boot::{EARLY_INFO, start_kernel};

    EARLY_INFO.call_once(|| mb2_info.to_early_boot_info());

    // SAFETY: The safety is guaranteed by the safety preconditions and the fact that we call it
    // once after setting up necessary resources.
    unsafe { start_kernel() };
}

#[cfg(ktest)]
mod test {
    use multiboot2::{Builder, FramebufferField, FramebufferTag, FramebufferType, MaybeDynSized};

    use super::*;
    use crate::prelude::ktest;

    #[ktest]
    fn multiboot2_framebuffer_preserves_pitch_and_channels() {
        for (red_pos, blue_pos) in [(0, 16), (16, 0)] {
            let buffer_type = FramebufferType::RGB {
                red: FramebufferField {
                    position: red_pos,
                    size: 8,
                },
                green: FramebufferField {
                    position: 8,
                    size: 8,
                },
                blue: FramebufferField {
                    position: blue_pos,
                    size: 8,
                },
            };
            let fb = parse_framebuffer(buffer_type.clone(), 16).unwrap();
            assert_eq!(fb.physical_range(), 0x1000..0x1020);
            assert_eq!((fb.width(), fb.height(), fb.bits_per_pixel()), (3, 2, 32));
            assert_eq!(fb.pitch_bytes(), 16);
            let layout = fb.rgb_layout().unwrap();
            assert_eq!(layout.red(), (red_pos, 8));
            assert_eq!(layout.green(), (8, 8));
            assert_eq!(layout.blue(), (blue_pos, 8));

            assert!(parse_framebuffer(buffer_type, 11).is_none());
        }
    }

    #[ktest]
    fn multiboot2_framebuffer_requires_rgb_metadata() {
        assert!(parse_framebuffer(FramebufferType::Text, 16).is_none());
        assert!(parse_framebuffer(FramebufferType::Indexed { palette: &[] }, 16).is_none());

        let structure = Builder::new().build();
        // SAFETY:
        // 1. The builder provides an aligned, initialized boot information structure.
        // 2. The structure remains alive for every use of `info`.
        let info = unsafe { BootInformation::load(structure.as_ptr()) }.unwrap();
        assert!(info.framebuffer_arg().is_none());
    }

    fn parse_framebuffer(
        buffer_type: FramebufferType<'_>,
        pitch_bytes: u32,
    ) -> Option<BootloaderFramebufferArg> {
        let structure = Builder::new()
            .framebuffer(FramebufferTag::new(
                0x1000,
                pitch_bytes,
                3,
                2,
                32,
                buffer_type,
            ))
            .build();
        // SAFETY:
        // 1. The builder provides an aligned, initialized boot information structure.
        // 2. The structure remains alive for every use of `info`.
        let info = unsafe { BootInformation::load(structure.as_ptr()) }.unwrap();
        info.framebuffer_arg()
    }
}
