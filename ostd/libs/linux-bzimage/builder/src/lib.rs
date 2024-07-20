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

use encoder::encode_kernel;
pub use encoder::PayloadEncoding;
use mapping::{SetupFileOffset, SetupVA};
use xmas_elf::program::SegmentData;

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
///  - `kernel_path`: The path to the kernel ELF;
///  - `setup_elf_path`: The path to the setup ELF;
///  - `encoding`: The encoding format for compressing the kernel ELF.
///
pub fn make_bzimage(
    target_image_path: &Path,
    image_type: BzImageType,
    kernel_path: &Path,
    setup_elf_path: &Path,
    encoding: PayloadEncoding,
) {
    let mut setup_elf = Vec::new();
    File::open(setup_elf_path)
        .unwrap()
        .read_to_end(&mut setup_elf)
        .unwrap();
    let mut setup = to_flat_binary(&setup_elf);
    // Pad the header with 8-byte alignment.
    setup.resize((setup.len() + 7) & !7, 0x00);

    let mut kernel = Vec::new();
    File::open(kernel_path)
        .unwrap()
        .read_to_end(&mut kernel)
        .unwrap();
    let payload = match image_type {
        BzImageType::Legacy32 => kernel,
        BzImageType::Efi64 => encode_kernel(kernel, encoding),
    };

    let setup_len = setup.len();
    let payload_len = payload.len();
    let payload_offset = SetupFileOffset::from(setup_len);
    fill_legacy_header_fields(&mut setup, payload_len, setup_len, payload_offset.into());

    let mut kernel_image = File::create(target_image_path).unwrap();
    kernel_image.write_all(&setup).unwrap();
    kernel_image.write_all(&payload).unwrap();

    let image_size = setup_len + payload_len;

    if matches!(image_type, BzImageType::Efi64) {
        // Write the PE/COFF header to the start of the file.
        // Since the Linux boot header starts at 0x1f1, we can write the PE/COFF header directly to the
        // start of the file without overwriting the Linux boot header.
        let pe_header = pe_header::make_pe_coff_header(&setup_elf, image_size);
        assert!(
            pe_header.header_at_zero.len() <= 0x1f1,
            "PE/COFF header is too large"
        );

        kernel_image.seek(SeekFrom::Start(0)).unwrap();
        kernel_image.write_all(&pe_header.header_at_zero).unwrap();
        kernel_image
            .seek(SeekFrom::Start(usize::from(pe_header.relocs.0) as u64))
            .unwrap();
        kernel_image.write_all(&pe_header.relocs.1).unwrap();
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
            let dst_file_length = program.file_size() as usize;
            if bin.len() < dst_file_offset + dst_file_length {
                bin.resize(dst_file_offset + dst_file_length, 0);
            }
            let dest_slice = bin[dst_file_offset..dst_file_offset + dst_file_length].as_mut();
            dest_slice.copy_from_slice(header_data);
        }
    }

    bin
}

/// This function should be used when generating the Linux x86 Boot setup header.
/// Some fields in the Linux x86 Boot setup header should be filled after assembled.
/// And the filled fields must have the bytes with values of 0xAB. See
/// `ostd/src/arch/x86/boot/linux_boot/setup/src/header.S` for more
/// info on this mechanism.
fn fill_header_field(header: &mut [u8], offset: usize, value: &[u8]) {
    let size = value.len();
    assert_eq!(
        &header[offset..offset + size],
        vec![0xABu8; size].as_slice(),
        "The field {:#x} to be filled must be marked with 0xAB",
        offset
    );
    header[offset..offset + size].copy_from_slice(value);
}

fn fill_legacy_header_fields(
    header: &mut [u8],
    kernel_len: usize,
    setup_len: usize,
    payload_offset: SetupVA,
) {
    fill_header_field(
        header,
        0x248, /* payload_offset */
        &(usize::from(payload_offset) as u32).to_le_bytes(),
    );

    fill_header_field(
        header,
        0x24C, /* payload_length */
        &(kernel_len as u32).to_le_bytes(),
    );

    fill_header_field(
        header,
        0x260, /* init_size */
        &((setup_len + kernel_len) as u32).to_le_bytes(),
    );
}
