// SPDX-License-Identifier: MPL-2.0

//! The linux bzImage builder.
//!
//! This crate is responsible for building the bzImage. It contains methods to build
//! the setup binary (with source provided in another crate) and methods to build the
//! bzImage from the setup binary and the kernel ELF.
//!
//! We should build the asterinas kernel as an ELF file, and feed it to the builder to
//! generate the bzImage. The builder will generate the PE/COFF header for the setup
//! code and concatenate it to the ELF file to make the bzImage.
//!
//! The setup code should be built into the ELF target and we convert it to a flat binary
//! in the builder.

pub mod encoder;
mod mapping;
mod pe_header;

use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    path::Path,
};

use align_ext::AlignExt;
pub use encoder::{encode_kernel, PayloadEncoding};
use mapping::{SetupFileOffset, SetupVA};
use xmas_elf::{program::SegmentData, sections::SectionData};

/// The type of the bzImage that we are building through `make_bzimage`.
///
/// Currently, Legacy32 and Efi64 are mutually exclusive.
pub enum BzImageType {
    Legacy32,
    Efi64,
}

/// Making a bzImage given the kernel ELF and setup source.
///
/// Explanations for the arguments:
///  - `target_image_path`: The path to the target bzImage;
///  - `image_type`: The type of the bzImage that we are building;
///  - `setup_elf_path`: The path to the setup ELF;
///
pub fn make_bzimage(target_image_path: &Path, image_type: BzImageType, setup_elf_path: &Path) {
    let mut setup_elf = Vec::new();
    File::open(setup_elf_path)
        .unwrap()
        .read_to_end(&mut setup_elf)
        .unwrap();
    let mut setup = to_flat_binary(&setup_elf);
    // Align the flat binary to `SECTION_ALIGNMENT`.
    setup.resize(setup.len().align_up(pe_header::SECTION_ALIGNMENT), 0x00);

    let mut kernel_image = File::create(target_image_path).unwrap();
    kernel_image.write_all(&setup).unwrap();

    if matches!(image_type, BzImageType::Efi64) {
        assert_elf64_reloc_supported(&setup_elf);

        // Write the PE/COFF header to the start of the file.
        // Since the Linux boot header starts at 0x1f1, we can write the PE/COFF header directly to the
        // start of the file without overwriting the Linux boot header.
        let pe_header = pe_header::make_pe_coff_header(&setup_elf);
        assert!(pe_header.len() <= 0x1f1, "PE/COFF header is too large");

        kernel_image.seek(SeekFrom::Start(0)).unwrap();
        kernel_image.write_all(&pe_header).unwrap();
    }
}

/// To build the legacy32 bzImage setup header, the OSDK should use this target.
pub fn legacy32_rust_target_json() -> &'static str {
    include_str!("x86_64-i386_pm-none.json")
}

/// We need a flat binary which satisfies PA delta == File offset delta,
/// and objcopy does not satisfy us well, so we should parse the ELF and
/// do our own objcopy job.
///
/// Interestingly, the resulting binary should be the same as the memory
/// dump of the kernel setup header when it's loaded by the bootloader.
fn to_flat_binary(elf_file: &[u8]) -> Vec<u8> {
    let elf = xmas_elf::ElfFile::new(elf_file).unwrap();
    let mut bin = Vec::<u8>::new();

    for program in elf.program_iter() {
        if program.get_type().unwrap() == xmas_elf::program::Type::Load {
            let SegmentData::Undefined(header_data) = program.get_data(&elf).unwrap() else {
                panic!("Unexpected segment data type");
            };
            let dst_file_offset = usize::from(SetupFileOffset::from(SetupVA::from(
                program.virtual_addr() as usize,
            )));

            // Note that `mem_size` can be greater than `file_size`. The remaining part must be
            // filled with zeros.
            let mem_length = program.mem_size() as usize;
            if bin.len() < dst_file_offset + mem_length {
                bin.resize(dst_file_offset + mem_length, 0);
            }

            // Copy the bytes in the `file_size` part.
            let file_length = program.file_size() as usize;
            let dest_slice = bin[dst_file_offset..dst_file_offset + file_length].as_mut();
            dest_slice.copy_from_slice(header_data);
        }
    }

    bin
}

fn assert_elf64_reloc_supported(elf_file: &[u8]) {
    const R_X86_64_RELATIVE: u32 = 8;

    let elf = xmas_elf::ElfFile::new(elf_file).unwrap();

    let SectionData::Rela64(rela64) = elf
        .find_section_by_name(".rela")
        .unwrap()
        .get_data(&elf)
        .unwrap()
    else {
        panic!("the ELF64 relocation data is not of the correct type");
    };

    rela64.iter().for_each(|r| {
        assert_eq!(
            r.get_type(),
            R_X86_64_RELATIVE,
            "the ELF64 relocation type is not supported"
        )
    });
}
