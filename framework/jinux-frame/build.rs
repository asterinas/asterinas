use std::{
    error::Error,
    io::{Seek, Write},
    path::{Path, PathBuf},
};

use xmas_elf::program::{ProgramHeader, SegmentData};

const SETUP32_LMA: usize = 0x100000;

fn main() -> Result<(), Box<dyn Error + Send + Sync>> {
    let source_dir = PathBuf::from(std::env::var("CARGO_MANIFEST_DIR").unwrap());
    let out_dir = PathBuf::from(std::env::var("OUT_DIR").unwrap());
    build_linux_setup_header(&source_dir, &out_dir)?;
    copy_to_raw_binary(&out_dir)?;
    Ok(())
}

fn build_linux_setup_header(
    source_dir: &Path,
    out_dir: &Path,
) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Build the setup header to ELF.
    let setup_crate_dir = source_dir
        .join("src")
        .join("arch")
        .join("x86")
        .join("boot")
        .join("linux_boot")
        .join("setup");
    let target_json = setup_crate_dir.join("x86_64-i386_protected_mode.json");

    println!(
        "cargo:rerun-if-changed={}",
        setup_crate_dir.to_str().unwrap()
    );

    let cargo = std::env::var("CARGO").unwrap();
    let mut cmd = std::process::Command::new(cargo);
    cmd.arg("install").arg("jinux-frame-x86-boot-setup");
    cmd.arg("--debug");
    cmd.arg("--locked");
    cmd.arg("--path").arg(setup_crate_dir.to_str().unwrap());
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
            "Failed to build linux x86 setup header::\n\tcommand `{:?}`\n\treturned {}",
            cmd, output.status
        )
        .into());
    }

    Ok(())
}

/// We need a binary which satisfies `LMA == File_Offset`, and objcopy
/// does not satisfy us well, so we should parse the ELF and do our own
/// objcopy job.
///
/// Interstingly, the resulting binary should be the same as the memory
/// dump of the kernel setup header when it's loaded by the bootloader.
fn copy_to_raw_binary(out_dir: &Path) -> Result<(), Box<dyn Error + Send + Sync>> {
    // Strip the elf header to get the raw header.
    let elf_path = out_dir.join("bin").join("jinux-frame-x86-boot-setup");
    let bin_path = out_dir.join("bin").join("jinux-frame-x86-boot-setup.bin");

    let elf_file = std::fs::read(elf_path)?;
    let elf = xmas_elf::ElfFile::new(&elf_file)?;

    let bin_file = std::fs::File::create(bin_path)?;
    let mut bin_writer = std::io::BufWriter::new(bin_file);

    for ph in elf.program_iter() {
        let ProgramHeader::Ph32(program) = ph else {
            return Err("Unexpected program header type".into());
        };
        if program.get_type().unwrap() == xmas_elf::program::Type::Load {
            let dest_file_offset = program.virtual_addr as usize - SETUP32_LMA;
            bin_writer.seek(std::io::SeekFrom::End(0))?;
            let cur_file_offset = bin_writer.stream_position().unwrap() as usize;
            if cur_file_offset < dest_file_offset {
                let padding = vec![0; dest_file_offset - cur_file_offset];
                bin_writer.write_all(&padding)?;
            } else {
                bin_writer.seek(std::io::SeekFrom::Start(dest_file_offset as u64))?;
            }
            let SegmentData::Undefined(header_data) = program.get_data(&elf).unwrap() else {
                return Err("Unexpected segment data type".into());
            };
            bin_writer.write_all(header_data)?;
        }
    }

    Ok(())
}
