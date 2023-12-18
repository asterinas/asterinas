use std::{
    fs::File,
    io::{Read, Write},
    path::Path,
};

use xmas_elf::program::{ProgramHeader, SegmentData};

// We chose the legacy setup sections to be 7 so that the setup header
// is page-aligned and the legacy setup section size would be 0x1000.
const LEGACY_SETUP_SECS: usize = 7;
const LEGACY_SETUP_SEC_SIZE: usize = 0x200 * (LEGACY_SETUP_SECS + 1);
const SETUP32_LMA: usize = 0x100000;

/// We need a binary which satisfies `LMA == File_Offset`, and objcopy
/// does not satisfy us well, so we should parse the ELF and do our own
/// objcopy job.
///
/// Interstingly, the resulting binary should be the same as the memory
/// dump of the kernel setup header when it's loaded by the bootloader.
fn header_to_raw_binary(elf_file: &[u8]) -> Vec<u8> {
    let elf = xmas_elf::ElfFile::new(&elf_file).unwrap();
    let mut bin = Vec::<u8>::new();

    for ph in elf.program_iter() {
        let ProgramHeader::Ph32(program) = ph else {
            panic!("Unexpected program header type");
        };
        if program.get_type().unwrap() == xmas_elf::program::Type::Load {
            let SegmentData::Undefined(header_data) = program.get_data(&elf).unwrap() else {
                panic!("Unexpected segment data type");
            };
            let dst_file_offset =
                program.virtual_addr as usize + LEGACY_SETUP_SEC_SIZE - SETUP32_LMA;
            let dst_file_length = program.file_size as usize;
            if bin.len() < dst_file_offset + dst_file_length {
                bin.resize(dst_file_offset + dst_file_length, 0);
            }
            let dest_slice = bin[dst_file_offset..dst_file_offset + dst_file_length].as_mut();
            dest_slice.copy_from_slice(header_data);
        }
    }

    bin
}

/// This function sould be used when generating the Linux x86 Boot setup header.
/// Some fields in the Linux x86 Boot setup header should be filled after assembled.
/// And the filled fields must have the bytes with values of 0xAB. See
/// `framework/jinux-frame/src/arch/x86/boot/linux_boot/setup/src/header.S` for more
/// info on this mechanism.
fn fill_header_field(header: &mut [u8], offset: usize, value: &[u8]) {
    let size = value.len();
    assert_eq!(
        &header[offset..offset + size],
        vec![0xABu8; size].as_slice()
    );
    header[offset..offset + size].copy_from_slice(value);
}

pub fn make_bzimage(path: &Path, kernel_path: &Path, header_path: &Path) -> std::io::Result<()> {
    let mut header = Vec::new();
    File::open(header_path)?.read_to_end(&mut header)?;
    let mut header = header_to_raw_binary(&header);
    // Pad the header to let the payload starts with 8-byte alignment.
    header.resize((header.len() + 7) & !7, 0x00);

    let mut kernel = Vec::new();
    File::open(kernel_path)?.read_to_end(&mut kernel)?;

    let header_len = header.len();
    let kernel_len = kernel.len();

    let payload_offset = header_len - LEGACY_SETUP_SEC_SIZE + SETUP32_LMA;
    fill_header_field(
        &mut header,
        0x248, /* payload_offset */
        &(payload_offset as u32).to_le_bytes(),
    );

    fill_header_field(
        &mut header,
        0x24C, /* payload_length */
        &(kernel_len as u32).to_le_bytes(),
    );

    fill_header_field(
        &mut header,
        0x260, /* init_size */
        &((header_len + kernel_len) as u32).to_le_bytes(),
    );

    let mut kernel_image = File::create(path)?;
    kernel_image.write_all(&header)?;
    kernel_image.write_all(&kernel)?;

    Ok(())
}
