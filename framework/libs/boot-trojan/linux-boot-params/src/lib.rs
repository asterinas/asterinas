//! The Linux Boot Protocol boot_params definition.
//!
//! The bootloader will deliver the address of the `BootParams` struct
//! as the argument of the kernel entrypoint. So we must define a Linux
//! ABI compatible struct in Rust, despite that most of the fields are
//! currently not needed by Asterinas.
//!

#![cfg_attr(not(test), no_std)]

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct ScreenInfo {
    pub orig_x: u8,             /* 0x00 */
    pub orig_y: u8,             /* 0x01 */
    pub ext_mem_k: u16,         /* 0x02 */
    pub orig_video_page: u16,   /* 0x04 */
    pub orig_video_mode: u8,    /* 0x06 */
    pub orig_video_cols: u8,    /* 0x07 */
    pub flags: u8,              /* 0x08 */
    pub unused2: u8,            /* 0x09 */
    pub orig_video_ega_bx: u16, /* 0x0a */
    pub unused3: u16,           /* 0x0c */
    pub orig_video_lines: u8,   /* 0x0e */
    pub orig_video_is_vga: u8,  /* 0x0f */
    pub orig_video_points: u16, /* 0x10 */

    /* VESA graphic mode -- linear frame buffer */
    pub lfb_width: u16,  /* 0x12 */
    pub lfb_height: u16, /* 0x14 */
    pub lfb_depth: u16,  /* 0x16 */
    pub lfb_base: u32,   /* 0x18 */
    pub lfb_size: u32,   /* 0x1c */
    pub cl_magic: u16,
    pub cl_offset: u16,       /* 0x20 */
    pub lfb_linelength: u16,  /* 0x24 */
    pub red_size: u8,         /* 0x26 */
    pub red_pos: u8,          /* 0x27 */
    pub green_size: u8,       /* 0x28 */
    pub green_pos: u8,        /* 0x29 */
    pub blue_size: u8,        /* 0x2a */
    pub blue_pos: u8,         /* 0x2b */
    pub rsvd_size: u8,        /* 0x2c */
    pub rsvd_pos: u8,         /* 0x2d */
    pub vesapm_seg: u16,      /* 0x2e */
    pub vesapm_off: u16,      /* 0x30 */
    pub pages: u16,           /* 0x32 */
    pub vesa_attributes: u16, /* 0x34 */
    pub capabilities: u32,    /* 0x36 */
    pub ext_lfb_base: u32,    /* 0x3a */
    pub _reserved: [u8; 2],   /* 0x3e */
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct ApmBiosInfo {
    pub version: u16,
    pub cseg: u16,
    pub offset: u32,
    pub cseg_16: u16,
    pub dseg: u16,
    pub flags: u16,
    pub cseg_len: u16,
    pub cseg_16_len: u16,
    pub dseg_len: u16,
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct IstInfo {
    pub signature: u32,
    pub command: u32,
    pub event: u32,
    pub perf_level: u32,
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct SysDescTable {
    pub length: u16,
    pub table: [u8; 14],
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct OlpcOfwHeader {
    pub ofw_magic: u32, /* OFW signature */
    pub ofw_version: u32,
    pub cif_handler: u32, /* callback into OFW */
    pub irq_desc_table: u32,
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct EdidInfo {
    pub dummy: [u8; 128],
}

#[derive(Copy, Clone, Debug)]
#[repr(C)]
pub struct EfiInfo {
    pub efi_loader_signature: u32,
    pub efi_systab: u32,
    pub efi_memdesc_size: u32,
    pub efi_memdesc_version: u32,
    pub efi_memmap: u32,
    pub efi_memmap_size: u32,
    pub efi_systab_hi: u32,
    pub efi_memmap_hi: u32,
}

/// Magic stored in SetupHeader.header.
pub const LINUX_BOOT_HEADER_MAGIC: u32 = 0x53726448;

/// Linux Boot Protocol Header.
///
/// Originally defined in the linux source tree:
/// `linux/arch/x86/include/uapi/asm/bootparam.h`
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct SetupHeader {
    pub setup_sects: u8,
    pub root_flags: u16,
    pub syssize: u32,
    pub ram_size: u16,
    pub vid_mode: u16,
    pub root_dev: u16,
    pub boot_flag: u16,
    pub jump: u16,
    pub header: u32,
    pub version: u16,
    pub realmode_swtch: u32,
    pub start_sys_seg: u16,
    pub kernel_version: u16,
    pub type_of_loader: u8,
    pub loadflags: u8,
    pub setup_move_size: u16,
    pub code32_start: u32,
    pub ramdisk_image: u32,
    pub ramdisk_size: u32,
    pub bootsect_kludge: u32,
    pub heap_end_ptr: u16,
    pub ext_loader_ver: u8,
    pub ext_loader_type: u8,
    pub cmd_line_ptr: u32,
    pub initrd_addr_max: u32,
    pub kernel_alignment: u32,
    pub relocatable_kernel: u8,
    pub min_alignment: u8,
    pub xloadflags: u16,
    pub cmdline_size: u32,
    pub hardware_subarch: u32,
    pub hardware_subarch_data: u64,
    pub payload_offset: u32,
    pub payload_length: u32,
    pub setup_data: u64,
    pub pref_address: u64,
    pub init_size: u32,
    pub handover_offset: u32,
    pub kernel_info_offset: u32,
}

/// The E820 types known to the kernel.
///
/// Originally defined in the linux source tree:
/// `linux/arch/x86/include/asm/e820/types.h`
#[derive(Copy, Clone, Debug)]
#[repr(u32)]
pub enum E820Type {
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
pub struct BootE820Entry {
    pub addr: u64,
    pub size: u64,
    pub typ: E820Type,
}

const E820_MAX_ENTRIES_ZEROPAGE: usize = 128;

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct EddDeviceParams {
    // TODO: We currently have no plans to support the edd device,
    // and we need unnamed fields (Rust RFC 2102) to implement this
    // FFI neatly. So we put a dummy implementation here conforming
    // to the BootParams struct ABI.
    pub _dummy: [u8; (0xeec - 0xd00) / 6 - 8],
}

#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct EddInfo {
    pub device: u8,
    pub version: u8,
    pub interface_support: u16,
    pub legacy_max_cylinder: u16,
    pub legacy_max_head: u8,
    pub legacy_sectors_per_track: u8,
    pub params: EddDeviceParams,
}

const EDD_MBR_SIG_MAX: usize = 16;
const EDDMAXNR: usize = 6;

/// Linux 32/64-bit Boot Protocol parameter struct.
///
/// Originally defined in the linux source tree:
/// `linux/arch/x86/include/uapi/asm/bootparam.h`
#[derive(Copy, Clone, Debug)]
#[repr(C, packed)]
pub struct BootParams {
    pub screen_info: ScreenInfo,        /* 0x000 */
    pub apm_bios_info: ApmBiosInfo,     /* 0x040 */
    pub _pad2: [u8; 4],                 /* 0x054 */
    pub tboot_addr: u64,                /* 0x058 */
    pub ist_info: IstInfo,              /* 0x060 */
    pub acpi_rsdp_addr: u64,            /* 0x070 */
    pub _pad3: [u8; 8],                 /* 0x078 */
    pub hd0_info: [u8; 16],             /* obsolete! 0x080 */
    pub hd1_info: [u8; 16],             /* obsolete! 0x090 */
    pub sys_desc_table: SysDescTable,   /* obsolete! 0x0a0 */
    pub olpc_ofw_header: OlpcOfwHeader, /* 0x0b0 */
    pub ext_ramdisk_image: u32,         /* 0x0c0 */
    pub ext_ramdisk_size: u32,          /* 0x0c4 */
    pub ext_cmd_line_ptr: u32,          /* 0x0c8 */
    pub _pad4: [u8; 112],               /* 0x0cc */
    pub cc_blob_address: u32,           /* 0x13c */
    pub edid_info: EdidInfo,            /* 0x140 */
    pub efi_info: EfiInfo,              /* 0x1c0 */
    pub alt_mem_k: u32,                 /* 0x1e0 */
    pub scratch: u32,                   /* Scratch field! 0x1e4 */
    pub e820_entries: u8,               /* 0x1e8 */
    pub eddbuf_entries: u8,             /* 0x1e9 */
    pub edd_mbr_sig_buf_entries: u8,    /* 0x1ea */
    pub kbd_status: u8,                 /* 0x1eb */
    pub secure_boot: u8,                /* 0x1ec */
    pub _pad5: [u8; 2],                 /* 0x1ed */
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
    pub sentinel: u8,     /* 0x1ef */
    pub _pad6: [u8; 1],   /* 0x1f0 */
    pub hdr: SetupHeader, /* setup header 0x1f1 */
    pub _pad7: [u8; 0x290 - 0x1f1 - core::mem::size_of::<SetupHeader>()],
    pub edd_mbr_sig_buffer: [u32; EDD_MBR_SIG_MAX], /* 0x290 */
    pub e820_table: [BootE820Entry; E820_MAX_ENTRIES_ZEROPAGE], /* 0x2d0 */
    pub _pad8: [u8; 48],                            /* 0xcd0 */
    pub eddbuf: [EddInfo; EDDMAXNR],                /* 0xd00 */
    pub _pad9: [u8; 276],                           /* 0xeec */
}
