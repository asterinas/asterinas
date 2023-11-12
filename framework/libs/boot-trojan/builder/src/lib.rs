mod mapping;
mod pe_header;

use std::{
    error::Error,
    fs::File,
    io::{Read, Seek, Write},
    path::Path,
};

use xmas_elf::program::{ProgramHeader, SegmentData};

use mapping::{TrojanFileOffset, TrojanVA};

/// We need a flat binary which satisfies PA delta == File delta, and objcopy
/// does not satisfy us well, so we should parse the ELF and do our own
/// objcopy job.
///
/// Interstingly, the resulting binary should be the same as the memory
/// dump of the kernel setup header when it's loaded by the bootloader.
fn trojan_to_flat_binary(elf_file: &[u8]) -> Vec<u8> {
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
            let dst_file_offset = usize::from(TrojanFileOffset::from(TrojanVA::from(
                program.virtual_addr as usize,
            )));
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
/// `framework/aster-frame/src/arch/x86/boot/linux_boot/setup/src/header.S` for more
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
    header_len: usize,
    payload_offset: TrojanVA,
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
        &((header_len + kernel_len) as u32).to_le_bytes(),
    );
}

pub fn make_bzimage(path: &Path, kernel_path: &Path, header_path: &Path) -> std::io::Result<()> {
    let mut header_elf_file = Vec::new();
    File::open(header_path)?.read_to_end(&mut header_elf_file)?;
    let mut header = trojan_to_flat_binary(&header_elf_file);
    // Pad the Linux boot header to let the payload starts with 8-byte alignment.
    header.resize((header.len() + 7) & !7, 0x00);

    let mut kernel = Vec::new();
    File::open(kernel_path)?.read_to_end(&mut kernel)?;
    let payload = kernel;

    let header_len = header.len();
    let payload_len = payload.len();
    let payload_offset = TrojanFileOffset::from(header_len);
    fill_legacy_header_fields(&mut header, payload_len, header_len, payload_offset.into());

    let mut kernel_image = File::create(path)?;
    kernel_image.write_all(&header)?;
    kernel_image.write_all(&payload)?;

    let image_size = header_len + payload_len;

    // Since the Linux boot header starts at 0x1f1, we can write the PE/COFF header directly to the
    // start of the file without overwriting the Linux boot header.
    let pe_header = pe_header::make_pe_coff_header(&header_elf_file, image_size);
    assert!(
        pe_header.header_at_zero.len() <= 0x1f1,
        "PE/COFF header is too large"
    );

    // FIXME: Oops, EFI hanover stucks, so I removed the pe header to let grub go through the legacy path.
    kernel_image.seek(std::io::SeekFrom::Start(0))?;
    // kernel_image.write_all(&pe_header.header_at_zero)?;
    kernel_image.seek(std::io::SeekFrom::Start(
        usize::from(pe_header.relocs.0) as u64
    ))?;
    // kernel_image.write_all(&pe_header.relocs.1)?;

    Ok(())
}

pub fn build_linux_setup_header_from_trojan(
    source_dir: &Path,
    out_dir: &Path,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Build the setup header to ELF.
    let target_json = source_dir.join("x86_64-i386_protected_mode.json");

    let cargo = std::env::var("CARGO").unwrap();
    let mut cmd = std::process::Command::new(cargo);
    cmd.arg("install").arg("aster-boot-trojan");
    cmd.arg("--debug");
    cmd.arg("--locked");
    cmd.arg("--path").arg(source_dir.to_str().unwrap());
    cmd.arg("--target").arg(target_json.as_os_str());
    cmd.arg("-Zbuild-std=core,compiler_builtins");
    cmd.arg("-Zbuild-std-features=compiler-builtins-mem");
    // Specify the installation root.
    cmd.arg("--root").arg(out_dir.as_os_str());
    // Specify the build target directory to avoid cargo running
    // into a deadlock reading the workspace files.
    cmd.arg("--target-dir").arg(out_dir.as_os_str());
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");
    let output = cmd.output()?;
    if !output.status.success() {
        std::io::stdout().write_all(&output.stdout).unwrap();
        std::io::stderr().write_all(&output.stderr).unwrap();
        return Err(format!(
            "Failed to build linux x86 setup header:\n\tcommand `{:?}`\n\treturned {}",
            cmd, output.status
        )
        .into());
    }

    Ok(())
}
