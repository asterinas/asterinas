// SPDX-License-Identifier: MPL-2.0

//! Big zImage PE/COFF header generation.
//!
//! The definition of the PE/COFF header is in the Microsoft PE/COFF specification:
//! https://learn.microsoft.com/en-us/windows/win32/debug/pe-format
//!
//! The reference to the Linux PE header definition:
//! https://github.com/torvalds/linux/blob/master/include/linux/pe.h

use bytemuck::{Pod, Zeroable};
use serde::Serialize;

use std::{mem::size_of, ops::Range};

use crate::mapping::{SetupFileOffset, SetupVA, LEGACY_SETUP_SEC_SIZE, SETUP32_LMA};

// The MS-DOS header.
const MZ_MAGIC: u16 = 0x5a4d; // "MZ"

// The `magic` field in PE header.
const PE_MAGIC: u32 = 0x00004550;

// The `machine` field choices in PE header. Not exhaustive.
#[derive(Serialize, Clone, Copy)]
#[repr(u16)]
enum PeMachineType {
    Amd64 = 0x8664,
}

// The `flags` field choices in PE header.
bitflags::bitflags! {
    struct PeFlags: u16 {
        const RELOCS_STRIPPED           = 1;
        const EXECUTABLE_IMAGE          = 1 << 1;
        const LINE_NUMS_STRIPPED        = 1 << 2;
        const LOCAL_SYMS_STRIPPED       = 1 << 3;
        const AGGRESIVE_WS_TRIM         = 1 << 4;
        const LARGE_ADDRESS_AWARE       = 1 << 5;
        const SIXTEEN_BIT_MACHINE       = 1 << 6;
        const BYTES_REVERSED_LO         = 1 << 7;
        const THIRTY_TWO_BIT_MACHINE    = 1 << 8;
        const DEBUG_STRIPPED            = 1 << 9;
        const REMOVABLE_RUN_FROM_SWAP   = 1 << 10;
        const NET_RUN_FROM_SWAP         = 1 << 11;
        const SYSTEM                    = 1 << 12;
        const DLL                       = 1 << 13;
        const UP_SYSTEM_ONLY            = 1 << 14;
    }
}

#[derive(Zeroable, Pod, Serialize, Clone, Copy)]
#[repr(C, packed)]
struct PeHdr {
    magic: u32,        // PE magic
    machine: u16,      // machine type
    sections: u16,     // number of sections
    timestamp: u32,    // time_t
    symbol_table: u32, // symbol table offset
    symbols: u32,      // number of symbols
    opt_hdr_size: u16, // size of optional header
    flags: u16,        // flags
}

// The `magic` field in the PE32+ optional header.
const PE32PLUS_OPT_HDR_MAGIC: u16 = 0x020b;

// The `subsys` field choices in the PE32+ optional header. Not exhaustive.
#[derive(Serialize, Clone, Copy)]
#[repr(u16)]
enum PeImageSubsystem {
    EfiApplication = 10,
}

#[derive(Zeroable, Pod, Serialize, Clone, Copy)]
#[repr(C, packed)]
struct Pe32PlusOptHdr {
    magic: u16,          // file type
    ld_major: u8,        // linker major version
    ld_minor: u8,        // linker minor version
    text_size: u32,      // size of text section(s)
    data_size: u32,      // size of data section(s)
    bss_size: u32,       // size of bss section(s)
    entry_point: u32,    // file offset of entry point
    code_base: u32,      // relative code addr in ram
    image_base: u64,     // preferred load address
    section_align: u32,  // alignment in bytes
    file_align: u32,     // file alignment in bytes
    os_major: u16,       // major OS version
    os_minor: u16,       // minor OS version
    image_major: u16,    // major image version
    image_minor: u16,    // minor image version
    subsys_major: u16,   // major subsystem version
    subsys_minor: u16,   // minor subsystem version
    win32_version: u32,  // reserved, must be 0
    image_size: u32,     // image size
    header_size: u32,    // header size rounded up to file_align
    csum: u32,           // checksum
    subsys: u16,         // subsystem
    dll_flags: u16,      // more flags!
    stack_size_req: u64, // amt of stack requested
    stack_size: u64,     // amt of stack required
    heap_size_req: u64,  // amt of heap requested
    heap_size: u64,      // amt of heap required
    loader_flags: u32,   // reserved, must be 0
    data_dirs: u32,      // number of data dir entries
}

#[derive(Zeroable, Pod, Serialize, Clone, Copy)]
#[repr(C, packed)]
struct Pe32PlusOptDataDirEnt {
    /// The RVA is the address of the table relative to the base address of the image when the table is loaded.
    rva: u32,
    size: u32,
}

impl Pe32PlusOptDataDirEnt {
    fn none() -> Self {
        Self { rva: 0, size: 0 }
    }
}

/// The data directories in the PE32+ optional header.
///
/// The `data_dirs` number field in the PE32+ optional header is just an illusion that you can choose to have a
/// subset of the data directories. The actual number of data directories is fixed to 16 and you can only ignore
/// data directories at the end of the list. We ignore data directories after the 8th as what Linux do.
#[derive(Zeroable, Pod, Serialize, Clone, Copy)]
#[repr(C, packed)]
struct Pe32PlusOptDataDirs {
    export_table: Pe32PlusOptDataDirEnt,
    import_table: Pe32PlusOptDataDirEnt,
    resource_table: Pe32PlusOptDataDirEnt,
    exception_table: Pe32PlusOptDataDirEnt,
    certificate_table: Pe32PlusOptDataDirEnt,
    base_relocation_table: Pe32PlusOptDataDirEnt,
}

impl Pe32PlusOptDataDirs {
    fn num_dirs() -> usize {
        size_of::<Self>() / size_of::<Pe32PlusOptDataDirEnt>()
    }
}

// The `flags` field choices in the PE section header.
// Excluding the alignment flags, which is not bitflags.
bitflags::bitflags! {
    struct PeSectionHdrFlags: u32 {
        const CNT_CODE                  = 1 << 5;
        const CNT_INITIALIZED_DATA      = 1 << 6;
        const CNT_UNINITIALIZED_DATA    = 1 << 7;
        const LNK_INFO                  = 1 << 9;
        const LNK_REMOVE                = 1 << 11;
        const LNK_COMDAT                = 1 << 12;
        const GPREL	                    = 1 << 15;
        const MEM_PURGEABLE             = 1 << 16;
        const LNK_NRELOC_OVFL           = 1 << 24;
        const MEM_DISCARDABLE           = 1 << 25;
        const MEM_NOT_CACHED            = 1 << 26;
        const MEM_NOT_PAGED             = 1 << 27;
        const MEM_SHARED                = 1 << 28;
        const MEM_EXECUTE               = 1 << 29;
        const MEM_READ                  = 1 << 30;
        const MEM_WRITE                 = 1 << 31;
    }
}

// The `flags` field choices in the PE section header.
#[derive(Serialize, Clone, Copy)]
#[repr(u32)]
enum PeSectionHdrFlagsAlign {
    _1Bytes = 0x00100000,
    _2Bytes = 0x00200000,
    _4Bytes = 0x00300000,
    _8Bytes = 0x00400000,
    _16Bytes = 0x00500000,
    _32Bytes = 0x00600000,
    _64Bytes = 0x00700000,
    _128Bytes = 0x00800000,
    _256Bytes = 0x00900000,
    _512Bytes = 0x00A00000,
    _1024Bytes = 0x00B00000,
    _2048Bytes = 0x00C00000,
    _4096Bytes = 0x00D00000,
    _8192Bytes = 0x00E00000,
}

#[derive(Zeroable, Pod, Serialize, Clone, Copy)]
#[repr(C, packed)]
struct PeSectionHdr {
    name: [u8; 8],        // name or "/12\0" string tbl offset
    virtual_size: u32,    // size of loaded section in RAM
    virtual_address: u32, // relative virtual address
    raw_data_size: u32,   // size of the section
    data_addr: u32,       // file pointer to first page of sec
    relocs: u32,          // file pointer to relocation entries
    line_numbers: u32,    // line numbers!
    num_relocs: u16,      // number of relocations
    num_lin_numbers: u16, // srsly.
    flags: u32,
}

struct ImageSectionAddrInfo {
    pub text: Range<SetupVA>,
    pub data: Range<SetupVA>,
    pub bss: Range<SetupVA>,
    /// All the readonly but loaded sections.
    pub rodata: Range<SetupVA>,
}

impl ImageSectionAddrInfo {
    fn from(elf: &xmas_elf::ElfFile) -> Self {
        let mut text_start = None;
        let mut text_end = None;
        let mut data_start = None;
        let mut data_end = None;
        let mut bss_start = None;
        let mut bss_end = None;
        let mut rodata_start = None;
        let mut rodata_end = None;
        for program in elf.program_iter() {
            if program.get_type().unwrap() == xmas_elf::program::Type::Load {
                let offset = SetupVA::from(program.virtual_addr() as usize);
                let length = program.mem_size() as usize;
                if program.flags().is_execute() {
                    text_start = Some(offset);
                    text_end = Some(offset + length);
                } else if program.flags().is_write() {
                    data_start = Some(offset);
                    data_end = Some(offset + program.file_size() as usize);
                    bss_start = Some(offset + program.file_size() as usize);
                    bss_end = Some(offset + length);
                } else if program.flags().is_read() {
                    rodata_start = Some(offset);
                    rodata_end = Some(offset + length);
                }
            }
        }

        Self {
            text: SetupVA::from(text_start.unwrap())..SetupVA::from(text_end.unwrap()),
            data: SetupVA::from(data_start.unwrap())..SetupVA::from(data_end.unwrap()),
            bss: SetupVA::from(bss_start.unwrap())..SetupVA::from(bss_end.unwrap()),
            rodata: SetupVA::from(rodata_start.unwrap())..SetupVA::from(rodata_end.unwrap()),
        }
    }

    fn text_virt_size(&self) -> usize {
        self.text.end - self.text.start
    }

    fn text_file_size(&self) -> usize {
        SetupFileOffset::from(self.text.end) - SetupFileOffset::from(self.text.start)
    }

    fn data_virt_size(&self) -> usize {
        self.data.end - self.data.start
    }

    fn data_file_size(&self) -> usize {
        SetupFileOffset::from(self.data.end) - SetupFileOffset::from(self.data.start)
    }

    fn bss_virt_size(&self) -> usize {
        self.bss.end - self.bss.start
    }

    fn rodata_virt_size(&self) -> usize {
        self.rodata.end - self.rodata.start
    }

    fn rodata_file_size(&self) -> usize {
        SetupFileOffset::from(self.rodata.end) - SetupFileOffset::from(self.rodata.start)
    }
}

pub struct ImagePeCoffHeaderBuf {
    pub header_at_zero: Vec<u8>,
    pub relocs: (SetupFileOffset, Vec<u8>),
}

pub(crate) fn make_pe_coff_header(setup_elf: &[u8], image_size: usize) -> ImagePeCoffHeaderBuf {
    let elf = xmas_elf::ElfFile::new(setup_elf).unwrap();
    let mut bin = Vec::<u8>::new();

    // The EFI application loader requires a relocation section.
    let relocs = vec![];
    // The place where we put the stub, must be after the legacy header and before 0x1000.
    let reloc_offset = SetupFileOffset::from(0x500);

    // PE header
    let mut pe_hdr = PeHdr {
        magic: PE_MAGIC,
        machine: PeMachineType::Amd64 as u16,
        sections: 0, // this field will be modified later
        timestamp: 0,
        symbol_table: 0,
        symbols: 1, // I don't know why, Linux header.S says it's 1
        opt_hdr_size: size_of::<Pe32PlusOptHdr>() as u16,
        flags: (PeFlags::EXECUTABLE_IMAGE | PeFlags::DEBUG_STRIPPED | PeFlags::LINE_NUMS_STRIPPED)
            .bits,
    };

    let elf_text_hdr = elf.find_section_by_name(".text").unwrap();

    // PE32+ optional header
    let pe_opt_hdr = Pe32PlusOptHdr {
        magic: PE32PLUS_OPT_HDR_MAGIC,
        ld_major: 0x02, // there's no linker to this extent, we do linking by ourselves
        ld_minor: 0x14,
        text_size: elf_text_hdr.size() as u32,
        data_size: 0, // data size is irrelevant
        bss_size: 0,  // bss size is irrelevant
        entry_point: elf.header.pt2.entry_point() as u32,
        code_base: elf_text_hdr.address() as u32,
        image_base: SETUP32_LMA as u64 - LEGACY_SETUP_SEC_SIZE as u64,
        section_align: 0x20,
        file_align: 0x20,
        os_major: 0,
        os_minor: 0,
        image_major: 0x3, // see linux/pe.h for more info
        image_minor: 0,
        subsys_major: 0,
        subsys_minor: 0,
        win32_version: 0,
        image_size: image_size as u32,
        header_size: LEGACY_SETUP_SEC_SIZE as u32,
        csum: 0,
        subsys: PeImageSubsystem::EfiApplication as u16,
        dll_flags: 0,
        stack_size_req: 0,
        stack_size: 0,
        heap_size_req: 0,
        heap_size: 0,
        loader_flags: 0,
        data_dirs: Pe32PlusOptDataDirs::num_dirs() as u32,
    };

    let pe_opt_hdr_data_dirs = Pe32PlusOptDataDirs {
        export_table: Pe32PlusOptDataDirEnt::none(),
        import_table: Pe32PlusOptDataDirEnt::none(),
        resource_table: Pe32PlusOptDataDirEnt::none(),
        exception_table: Pe32PlusOptDataDirEnt::none(),
        certificate_table: Pe32PlusOptDataDirEnt::none(),
        base_relocation_table: Pe32PlusOptDataDirEnt {
            rva: usize::from(reloc_offset) as u32,
            size: relocs.len() as u32,
        },
    };

    let addr_info = ImageSectionAddrInfo::from(&elf);

    // PE section headers
    let mut sec_hdrs = Vec::<PeSectionHdr>::new();
    // .reloc
    sec_hdrs.push(PeSectionHdr {
        name: [b'.', b'r', b'e', b'l', b'o', b'c', 0, 0],
        virtual_size: relocs.len() as u32,
        virtual_address: usize::from(SetupVA::from(reloc_offset)) as u32,
        raw_data_size: relocs.len() as u32,
        data_addr: usize::from(reloc_offset) as u32,
        relocs: 0,
        line_numbers: 0,
        num_relocs: 0,
        num_lin_numbers: 0,
        flags: (PeSectionHdrFlags::CNT_INITIALIZED_DATA
            | PeSectionHdrFlags::MEM_READ
            | PeSectionHdrFlags::MEM_DISCARDABLE)
            .bits
            | PeSectionHdrFlagsAlign::_1Bytes as u32,
    });
    // .text
    sec_hdrs.push(PeSectionHdr {
        name: [b'.', b't', b'e', b'x', b't', 0, 0, 0],
        virtual_size: addr_info.text_virt_size() as u32,
        virtual_address: usize::from(addr_info.text.start) as u32,
        raw_data_size: addr_info.text_file_size() as u32,
        data_addr: usize::from(SetupFileOffset::from(addr_info.text.start)) as u32,
        relocs: 0,
        line_numbers: 0,
        num_relocs: 0,
        num_lin_numbers: 0,
        flags: (PeSectionHdrFlags::CNT_CODE
            | PeSectionHdrFlags::MEM_READ
            | PeSectionHdrFlags::MEM_EXECUTE)
            .bits
            | PeSectionHdrFlagsAlign::_16Bytes as u32,
    });
    // .data
    sec_hdrs.push(PeSectionHdr {
        name: [b'.', b'd', b'a', b't', b'a', 0, 0, 0],
        virtual_size: addr_info.data_virt_size() as u32,
        virtual_address: usize::from(addr_info.data.start) as u32,
        raw_data_size: addr_info.data_file_size() as u32,
        data_addr: usize::from(SetupFileOffset::from(addr_info.data.start)) as u32,
        relocs: 0,
        line_numbers: 0,
        num_relocs: 0,
        num_lin_numbers: 0,
        flags: (PeSectionHdrFlags::CNT_INITIALIZED_DATA
            | PeSectionHdrFlags::MEM_READ
            | PeSectionHdrFlags::MEM_WRITE)
            .bits
            | PeSectionHdrFlagsAlign::_16Bytes as u32,
    });
    // .bss
    sec_hdrs.push(PeSectionHdr {
        name: [b'.', b'b', b's', b's', 0, 0, 0, 0],
        virtual_size: addr_info.bss_virt_size() as u32,
        virtual_address: usize::from(addr_info.bss.start) as u32,
        raw_data_size: 0,
        data_addr: 0,
        relocs: 0,
        line_numbers: 0,
        num_relocs: 0,
        num_lin_numbers: 0,
        flags: (PeSectionHdrFlags::CNT_UNINITIALIZED_DATA
            | PeSectionHdrFlags::MEM_READ
            | PeSectionHdrFlags::MEM_WRITE)
            .bits
            | PeSectionHdrFlagsAlign::_16Bytes as u32,
    });
    // .rodata
    sec_hdrs.push(PeSectionHdr {
        name: [b'.', b'r', b'o', b'd', b'a', b't', b'a', 0],
        virtual_size: addr_info.rodata_virt_size() as u32,
        virtual_address: usize::from(addr_info.rodata.start) as u32,
        raw_data_size: addr_info.rodata_file_size() as u32,
        data_addr: usize::from(SetupFileOffset::from(addr_info.rodata.start)) as u32,
        relocs: 0,
        line_numbers: 0,
        num_relocs: 0,
        num_lin_numbers: 0,
        flags: (PeSectionHdrFlags::CNT_INITIALIZED_DATA | PeSectionHdrFlags::MEM_READ).bits
            | PeSectionHdrFlagsAlign::_16Bytes as u32,
    });

    // Write the MS-DOS header
    bin.extend_from_slice(&MZ_MAGIC.to_le_bytes());
    // Write the MS-DOS stub at 0x3c
    bin.extend_from_slice(&[0x0; 0x3c - 0x2]);
    // Write the PE header offset, the header is right after the offset field
    bin.extend_from_slice(&(0x3cu32 + size_of::<u32>() as u32).to_le_bytes());

    // Write the PE header
    pe_hdr.sections = sec_hdrs.len() as u16;
    bin.extend_from_slice(bytemuck::bytes_of(&pe_hdr));
    // Write the PE32+ optional header
    bin.extend_from_slice(bytemuck::bytes_of(&pe_opt_hdr));
    bin.extend_from_slice(bytemuck::bytes_of(&pe_opt_hdr_data_dirs));
    // Write the PE section headers
    for sec_hdr in sec_hdrs {
        bin.extend_from_slice(bytemuck::bytes_of(&sec_hdr));
    }

    ImagePeCoffHeaderBuf {
        header_at_zero: bin,
        relocs: (reloc_offset, relocs),
    }
}
