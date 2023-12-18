//! The Linux Boot Protocol boot_params module.
//!
//! The bootloader will deliver the address of the `BootParams` struct
//! as the argument of the kernel entrypoint. So we must define a Linux
//! ABI compatible struct in Rust, despite that most of the fields are
//! currently not needed by Jinux.
//!

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct ScreenInfo {
    pub(super) orig_x: u8,             /* 0x00 */
    pub(super) orig_y: u8,             /* 0x01 */
    pub(super) ext_mem_k: u16,         /* 0x02 */
    pub(super) orig_video_page: u16,   /* 0x04 */
    pub(super) orig_video_mode: u8,    /* 0x06 */
    pub(super) orig_video_cols: u8,    /* 0x07 */
    pub(super) flags: u8,              /* 0x08 */
    pub(super) unused2: u8,            /* 0x09 */
    pub(super) orig_video_ega_bx: u16, /* 0x0a */
    pub(super) unused3: u16,           /* 0x0c */
    pub(super) orig_video_lines: u8,   /* 0x0e */
    pub(super) orig_video_is_vga: u8,  /* 0x0f */
    pub(super) orig_video_points: u16, /* 0x10 */

    /* VESA graphic mode -- linear frame buffer */
    pub(super) lfb_width: u16,  /* 0x12 */
    pub(super) lfb_height: u16, /* 0x14 */
    pub(super) lfb_depth: u16,  /* 0x16 */
    pub(super) lfb_base: u32,   /* 0x18 */
    pub(super) lfb_size: u32,   /* 0x1c */
    pub(super) cl_magic: u16,
    pub(super) cl_offset: u16,       /* 0x20 */
    pub(super) lfb_linelength: u16,  /* 0x24 */
    pub(super) red_size: u8,         /* 0x26 */
    pub(super) red_pos: u8,          /* 0x27 */
    pub(super) green_size: u8,       /* 0x28 */
    pub(super) green_pos: u8,        /* 0x29 */
    pub(super) blue_size: u8,        /* 0x2a */
    pub(super) blue_pos: u8,         /* 0x2b */
    pub(super) rsvd_size: u8,        /* 0x2c */
    pub(super) rsvd_pos: u8,         /* 0x2d */
    pub(super) vesapm_seg: u16,      /* 0x2e */
    pub(super) vesapm_off: u16,      /* 0x30 */
    pub(super) pages: u16,           /* 0x32 */
    pub(super) vesa_attributes: u16, /* 0x34 */
    pub(super) capabilities: u32,    /* 0x36 */
    pub(super) ext_lfb_base: u32,    /* 0x3a */
    pub(super) _reserved: [u8; 2],   /* 0x3e */
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct ApmBiosInfo {
    pub(super) version: u16,
    pub(super) cseg: u16,
    pub(super) offset: u32,
    pub(super) cseg_16: u16,
    pub(super) dseg: u16,
    pub(super) flags: u16,
    pub(super) cseg_len: u16,
    pub(super) cseg_16_len: u16,
    pub(super) dseg_len: u16,
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct IstInfo {
    pub(super) signature: u32,
    pub(super) command: u32,
    pub(super) event: u32,
    pub(super) perf_level: u32,
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct SysDescTable {
    pub(super) length: u16,
    pub(super) table: [u8; 14],
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct OlpcOfwHeader {
    pub(super) ofw_magic: u32, /* OFW signature */
    pub(super) ofw_version: u32,
    pub(super) cif_handler: u32, /* callback into OFW */
    pub(super) irq_desc_table: u32,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub(super) struct EdidInfo {
    pub(super) dummy: [u8; 128],
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub(super) struct EfiInfo {
    pub(super) efi_loader_signature: u32,
    pub(super) efi_systab: u32,
    pub(super) efi_memdesc_size: u32,
    pub(super) efi_memdesc_version: u32,
    pub(super) efi_memmap: u32,
    pub(super) efi_memmap_size: u32,
    pub(super) efi_systab_hi: u32,
    pub(super) efi_memmap_hi: u32,
}

/// Magic stored in SetupHeader.header.
pub(super) const LINUX_BOOT_HEADER_MAGIC: u32 = 0x53726448;

/// Linux Boot Protocol Header.
///
/// Originally defined in the linux source tree:
/// `linux/arch/x86/include/uapi/asm/bootparam.h`
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct SetupHeader {
    pub(super) setup_sects: u8,
    pub(super) root_flags: u16,
    pub(super) syssize: u32,
    pub(super) ram_size: u16,
    pub(super) vid_mode: u16,
    pub(super) root_dev: u16,
    pub(super) boot_flag: u16,
    pub(super) jump: u16,
    pub(super) header: u32,
    pub(super) version: u16,
    pub(super) realmode_swtch: u32,
    pub(super) start_sys_seg: u16,
    pub(super) kernel_version: u16,
    pub(super) type_of_loader: u8,
    pub(super) loadflags: u8,
    pub(super) setup_move_size: u16,
    pub(super) code32_start: u32,
    pub(super) ramdisk_image: u32,
    pub(super) ramdisk_size: u32,
    pub(super) bootsect_kludge: u32,
    pub(super) heap_end_ptr: u16,
    pub(super) ext_loader_ver: u8,
    pub(super) ext_loader_type: u8,
    pub(super) cmd_line_ptr: u32,
    pub(super) initrd_addr_max: u32,
    pub(super) kernel_alignment: u32,
    pub(super) relocatable_kernel: u8,
    pub(super) min_alignment: u8,
    pub(super) xloadflags: u16,
    pub(super) cmdline_size: u32,
    pub(super) hardware_subarch: u32,
    pub(super) hardware_subarch_data: u64,
    pub(super) payload_offset: u32,
    pub(super) payload_length: u32,
    pub(super) setup_data: u64,
    pub(super) pref_address: u64,
    pub(super) init_size: u32,
    pub(super) handover_offset: u32,
    pub(super) kernel_info_offset: u32,
}

/// The E820 types known to the kernel.
///
/// Originally defined in the linux source tree:
/// `linux/arch/x86/include/asm/e820/types.h`
#[derive(Copy, Clone, Debug)]
#[repr(u32)]
pub(super) enum E820Type {
    Ram = 1,
    Reserved = 2,
    Acpi = 3,
    Nvs = 4,
    Unusable = 5,
    Pmem = 7,
    /*
     * This is a non-standardized way to represent ADR or
     * NVDIMM regions that persist over a reboot.
     *
     * The kernel will ignore their special capabilities
     * unless the CONFIG_X86_PMEM_LEGACY=y option is set.
     *
     * ( Note that older platforms also used 6 for the same
     *   type of memory, but newer versions switched to 12 as
     *   6 was assigned differently. Some time they will learn... )
     */
    Pram = 12,
    /*
     * Special-purpose memory is indicated to the system via the
     * EFI_MEMORY_SP attribute. Define an e820 translation of this
     * memory type for the purpose of reserving this range and
     * marking it with the IORES_DESC_SOFT_RESERVED designation.
     */
    SoftReserved = 0xefffffff,
    /*
     * Reserved RAM used by the kernel itself if
     * CONFIG_INTEL_TXT=y is enabled, memory of this type
     * will be included in the S3 integrity calculation
     * and so should not include any memory that the BIOS
     * might alter over the S3 transition:
     */
    ReservedKern = 128,
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct BootE820Entry {
    pub(super) addr: u64,
    pub(super) size: u64,
    pub(super) typ: E820Type,
}

const E820_MAX_ENTRIES_ZEROPAGE: usize = 128;

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct EddDeviceParams {
    // TODO: We currently have no plans to support the edd device,
    // and we need unnamed fields (Rust RFC 2102) to implement this
    // FFI neatly. So we put a dummy implementation here conforming
    // to the BootParams struct ABI.
    pub(super) _dummy: [u8; (0xeec - 0xd00) / 6 - 8],
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct EddInfo {
    pub(super) device: u8,
    pub(super) version: u8,
    pub(super) interface_support: u16,
    pub(super) legacy_max_cylinder: u16,
    pub(super) legacy_max_head: u8,
    pub(super) legacy_sectors_per_track: u8,
    pub(super) params: EddDeviceParams,
}

const EDD_MBR_SIG_MAX: usize = 16;
const EDDMAXNR: usize = 6;

/// Linux 32/64-bit Boot Protocol parameter struct.
///
/// Originally defined in the linux source tree:
/// `linux/arch/x86/include/uapi/asm/bootparam.h`
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub(super) struct BootParams {
    pub(super) screen_info: ScreenInfo,        /* 0x000 */
    pub(super) apm_bios_info: ApmBiosInfo,     /* 0x040 */
    pub(super) _pad2: [u8; 4],                 /* 0x054 */
    pub(super) tboot_addr: u64,                /* 0x058 */
    pub(super) ist_info: IstInfo,              /* 0x060 */
    pub(super) acpi_rsdp_addr: u64,            /* 0x070 */
    pub(super) _pad3: [u8; 8],                 /* 0x078 */
    pub(super) hd0_info: [u8; 16],             /* obsolete! 0x080 */
    pub(super) hd1_info: [u8; 16],             /* obsolete! 0x090 */
    pub(super) sys_desc_table: SysDescTable,   /* obsolete! 0x0a0 */
    pub(super) olpc_ofw_header: OlpcOfwHeader, /* 0x0b0 */
    pub(super) ext_ramdisk_image: u32,         /* 0x0c0 */
    pub(super) ext_ramdisk_size: u32,          /* 0x0c4 */
    pub(super) ext_cmd_line_ptr: u32,          /* 0x0c8 */
    pub(super) _pad4: [u8; 112],               /* 0x0cc */
    pub(super) cc_blob_address: u32,           /* 0x13c */
    pub(super) edid_info: EdidInfo,            /* 0x140 */
    pub(super) efi_info: EfiInfo,              /* 0x1c0 */
    pub(super) alt_mem_k: u32,                 /* 0x1e0 */
    pub(super) scratch: u32,                   /* Scratch field! 0x1e4 */
    pub(super) e820_entries: u8,               /* 0x1e8 */
    pub(super) eddbuf_entries: u8,             /* 0x1e9 */
    pub(super) edd_mbr_sig_buf_entries: u8,    /* 0x1ea */
    pub(super) kbd_status: u8,                 /* 0x1eb */
    pub(super) secure_boot: u8,                /* 0x1ec */
    pub(super) _pad5: [u8; 2],                 /* 0x1ed */
    /*
     * The sentinel is set to a nonzero value (0xff) in header.S.
     *
     * A bootloader is supposed to only take setup_header and put
     * it into a clean boot_params buffer. If it turns out that
     * it is clumsy or too generous with the buffer, it most
     * probably will pick up the sentinel variable too. The fact
     * that this variable then is still 0xff will let kernel
     * know that some variables in boot_params are invalid and
     * kernel should zero out certain portions of boot_params.
     */
    pub(super) sentinel: u8,     /* 0x1ef */
    pub(super) _pad6: [u8; 1],   /* 0x1f0 */
    pub(super) hdr: SetupHeader, /* setup header 0x1f1 */
    pub(super) _pad7: [u8; 0x290 - 0x1f1 - core::mem::size_of::<SetupHeader>()],
    pub(super) edd_mbr_sig_buffer: [u32; EDD_MBR_SIG_MAX], /* 0x290 */
    pub(super) e820_table: [BootE820Entry; E820_MAX_ENTRIES_ZEROPAGE], /* 0x2d0 */
    pub(super) _pad8: [u8; 48],                            /* 0xcd0 */
    pub(super) eddbuf: [EddInfo; EDDMAXNR],                /* 0xd00 */
    pub(super) _pad9: [u8; 276],                           /* 0xeec */
}
