mod mapping;
mod pe_header;

use std::{
    fs::File,
    io::{Read, Write},
    path::{Path, PathBuf},
};

use xmas_elf::program::SegmentData;

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

    for program in elf.program_iter() {
        if program.get_type().unwrap() == xmas_elf::program::Type::Load {
            let SegmentData::Undefined(header_data) = program.get_data(&elf).unwrap() else {
                panic!("Unexpected segment data type");
            };
            let dst_file_offset = usize::from(TrojanFileOffset::from(TrojanVA::from(
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
    trojan_len: usize,
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
        &((trojan_len + kernel_len) as u32).to_le_bytes(),
    );
}

pub fn make_bzimage(path: &Path, kernel_path: &Path, trojan_src: &Path, trojan_out: &Path) {
    #[cfg(feature = "trojan64")]
    let trojan = build_trojan_with_arch(trojan_src, trojan_out, &TrojanBuildArch::X86_64);

    #[cfg(not(feature = "trojan64"))]
    let trojan = {
        let arch = trojan_src
            .join("x86_64-i386_pm-none.json")
            .canonicalize()
            .unwrap();
        build_trojan_with_arch(trojan_src, trojan_out, &TrojanBuildArch::Other(arch))
    };

    let mut trojan_elf = Vec::new();
    File::open(trojan)
        .unwrap()
        .read_to_end(&mut trojan_elf)
        .unwrap();
    let mut trojan = trojan_to_flat_binary(&trojan_elf);
    // Pad the header with 8-byte alignment.
    trojan.resize((trojan.len() + 7) & !7, 0x00);

    let mut kernel = Vec::new();
    File::open(kernel_path)
        .unwrap()
        .read_to_end(&mut kernel)
        .unwrap();
    let payload = kernel;

    let trojan_len = trojan.len();
    let payload_len = payload.len();
    let payload_offset = TrojanFileOffset::from(trojan_len);
    fill_legacy_header_fields(&mut trojan, payload_len, trojan_len, payload_offset.into());

    let mut kernel_image = File::create(path).unwrap();
    kernel_image.write_all(&trojan).unwrap();
    kernel_image.write_all(&payload).unwrap();

    let image_size = trojan_len + payload_len;

    // Since the Linux boot header starts at 0x1f1, we can write the PE/COFF header directly to the
    // start of the file without overwriting the Linux boot header.
    let pe_header = pe_header::make_pe_coff_header(&trojan_elf, image_size);
    assert!(
        pe_header.header_at_zero.len() <= 0x1f1,
        "PE/COFF header is too large"
    );

    #[cfg(feature = "trojan64")]
    {
        use std::io::{Seek, SeekFrom};
        kernel_image.seek(SeekFrom::Start(0)).unwrap();
        kernel_image.write_all(&pe_header.header_at_zero).unwrap();
        kernel_image
            .seek(SeekFrom::Start(usize::from(pe_header.relocs.0) as u64))
            .unwrap();
        kernel_image.write_all(&pe_header.relocs.1).unwrap();
    }
}

// We need a custom target file for i386 but not for x86_64.
// The compiler may warn us the X86_64 enum variant is not constructed
// when we are building for i386, but we can ignore it.
#[allow(dead_code)]
enum TrojanBuildArch {
    X86_64,
    Other(PathBuf),
}

fn build_trojan_with_arch(source_dir: &Path, out_dir: &Path, arch: &TrojanBuildArch) -> PathBuf {
    if !out_dir.exists() {
        std::fs::create_dir_all(&out_dir).unwrap();
    }
    let out_dir = std::fs::canonicalize(out_dir).unwrap();

    let cargo = std::env::var("CARGO").unwrap();
    let mut cmd = std::process::Command::new(cargo);
    cmd.current_dir(source_dir);
    cmd.arg("build");
    // Relocations are fewer in release mode, saving header real-estate.
    cmd.arg("--release");
    cmd.arg("--package").arg("aster-boot-trojan");
    cmd.arg("--target").arg(match arch {
        TrojanBuildArch::X86_64 => "x86_64-unknown-none",
        TrojanBuildArch::Other(path) => path.to_str().unwrap(),
    });
    cmd.arg("-Zbuild-std=core,alloc,compiler_builtins");
    cmd.arg("-Zbuild-std-features=compiler-builtins-mem");
    // Specify the build target directory to avoid cargo running
    // into a deadlock reading the workspace files.
    cmd.arg("--target-dir").arg(out_dir.as_os_str());
    cmd.env_remove("RUSTFLAGS");
    cmd.env_remove("CARGO_ENCODED_RUSTFLAGS");

    let mut child = cmd.spawn().unwrap();
    let status = child.wait().unwrap();
    if !status.success() {
        panic!(
            "Failed to build linux x86 setup header:\n\tcommand `{:?}`\n\treturned {}",
            cmd, status
        );
    }

    // Return the path to the trojan binary.
    let arch_name = match arch {
        TrojanBuildArch::X86_64 => "x86_64-unknown-none",
        TrojanBuildArch::Other(path) => path.file_stem().unwrap().to_str().unwrap(),
    };

    let trojan_artifact = out_dir
        .join(arch_name)
        .join("release")
        .join("aster-boot-trojan");

    trojan_artifact.to_owned()
}
