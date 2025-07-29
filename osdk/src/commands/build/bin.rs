// SPDX-License-Identifier: MPL-2.0

use std::{
    fs::{File, OpenOptions},
    io::{Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
};

use linux_bzimage_builder::{
    encode_kernel, legacy32_rust_target_json, make_bzimage, BzImageType, PayloadEncoding,
};

use crate::{
    arch::Arch,
    bundle::{
        bin::{AsterBin, AsterBinType, AsterBzImageMeta, AsterElfMeta},
        file::BundleFile,
    },
    util::{get_current_crates, hard_link_or_copy, new_command_checked_exists},
};

pub fn make_install_bzimage(
    install_dir: impl AsRef<Path>,
    target_dir: impl AsRef<Path>,
    aster_elf: &AsterBin,
    linux_x86_legacy_boot: bool,
    encoding: PayloadEncoding,
) -> AsterBin {
    let target_name = get_current_crates().remove(0).name;
    let image_type = if linux_x86_legacy_boot {
        BzImageType::Legacy32
    } else {
        BzImageType::Efi64
    };
    let setup_bin = {
        let setup_install_dir = target_dir.as_ref();
        let setup_target_dir = &target_dir.as_ref().join("linux-bzimage-setup");
        match image_type {
            BzImageType::Legacy32 => {
                let target_json = legacy32_rust_target_json();
                let gen_target_json_path = target_dir.as_ref().join("x86_64-i386_pm-none.json");
                std::fs::write(&gen_target_json_path, target_json).unwrap();
                let arch = gen_target_json_path.canonicalize().unwrap();
                install_setup_with_arch(
                    setup_install_dir,
                    setup_target_dir,
                    arch.to_str().unwrap(),
                    aster_elf,
                    encoding,
                );
            }
            BzImageType::Efi64 => {
                install_setup_with_arch(
                    setup_install_dir,
                    setup_target_dir,
                    "x86_64-unknown-none",
                    aster_elf,
                    encoding,
                );
            }
        };
        setup_install_dir.join("bin").join("linux-bzimage-setup")
    };
    // Make the `bzImage`-compatible kernel image and place it in the boot directory.
    let install_path = install_dir.as_ref().join(target_name);
    info!("Building bzImage");
    println!("install_path: {:?}", install_path);
    make_bzimage(&install_path, image_type, &setup_bin);

    AsterBin::new(
        &install_path,
        aster_elf.arch(),
        AsterBinType::BzImage(AsterBzImageMeta {
            support_legacy32_boot: linux_x86_legacy_boot,
            support_efi_boot: false,
            support_efi_handover: !linux_x86_legacy_boot,
        }),
        aster_elf.version().clone(),
        aster_elf.stripped(),
    )
}

pub fn make_elf_for_qemu(install_dir: impl AsRef<Path>, elf: &AsterBin, strip: bool) -> AsterBin {
    let result_elf_path = {
        let elf_name = elf
            .path()
            .file_name()
            .unwrap()
            .to_str()
            .unwrap()
            .to_string();
        install_dir.as_ref().join(elf_name + ".qemu_elf")
    };

    if strip {
        // We use rust-strip to reduce the kernel image size.
        let status = new_command_checked_exists("rust-strip")
            .arg(elf.path())
            .arg("-o")
            .arg(result_elf_path.as_os_str())
            .status();

        match status {
            Ok(status) => {
                if !status.success() {
                    panic!("Failed to strip kernel elf.");
                }
            }
            Err(err) => match err.kind() {
                std::io::ErrorKind::NotFound => panic!(
                    "`rust-strip` command not found. Please
                    try `cargo install cargo-binutils` and then rerun."
                ),
                _ => panic!("Strip kernel elf failed, err:{:#?}", err),
            },
        }
    } else {
        // Copy the ELF file.
        hard_link_or_copy(elf.path(), &result_elf_path).unwrap();
    }

    if elf.arch() == Arch::X86_64 {
        // Because QEMU denies a x86_64 multiboot ELF file (GRUB2 accept it, btw),
        // modify `em_machine` to pretend to be an x86 (32-bit) ELF image,
        //
        // https://github.com/qemu/qemu/blob/950c4e6c94b15cd0d8b63891dddd7a8dbf458e6a/hw/i386/multiboot.c#L197
        // Set EM_386 (0x0003) to em_machine.
        let mut file = OpenOptions::new()
            .read(true)
            .write(true)
            .open(&result_elf_path)
            .unwrap();

        let bytes: [u8; 2] = [0x03, 0x00];

        file.seek(SeekFrom::Start(18)).unwrap();
        file.write_all(&bytes).unwrap();
        file.flush().unwrap();
    }

    AsterBin::new(
        &result_elf_path,
        elf.arch(),
        AsterBinType::Elf(AsterElfMeta {
            has_linux_header: false,
            has_pvh_header: false,
            has_multiboot_header: true,
            has_multiboot2_header: true,
        }),
        elf.version().clone(),
        strip,
    )
}

fn install_setup_with_arch(
    install_dir: impl AsRef<Path>,
    target_dir: impl AsRef<Path>,
    arch: &str,
    aster_elf: &AsterBin,
    encoding: PayloadEncoding,
) {
    if !target_dir.as_ref().exists() {
        std::fs::create_dir_all(&target_dir).unwrap();
    }
    let target_dir = std::fs::canonicalize(target_dir).unwrap();

    let mut cmd = new_command_checked_exists("cargo");
    let rustflags = [
        "-Cdebuginfo=2",
        "-Ccode-model=kernel",
        "-Crelocation-model=pie",
        "-Zplt=yes",
        "-Zrelax-elf-relocations=yes",
        "-Crelro-level=full",
        "-Ctarget-feature=+crt-static",
    ];
    cmd.env("RUSTFLAGS", rustflags.join(" "));
    cmd.env("PAYLOAD_FILE", encode_kernel_to_file(aster_elf, encoding));
    cmd.arg("install").arg("linux-bzimage-setup");
    cmd.arg("--force");
    cmd.arg("--root").arg(install_dir.as_ref());
    if matches!(option_env!("OSDK_LOCAL_DEV"), Some("1")) {
        let crate_dir = PathBuf::from(env!("CARGO_MANIFEST_DIR"));
        let setup_dir = crate_dir.join("../ostd/libs/linux-bzimage/setup");
        cmd.arg("--path").arg(setup_dir);
    } else {
        cmd.arg("--version").arg(env!("CARGO_PKG_VERSION"));
    }
    cmd.arg("--target").arg(arch);
    cmd.arg("-Zbuild-std=core,alloc,compiler_builtins");
    cmd.arg("-Zbuild-std-features=compiler-builtins-mem");
    // Specify the build target directory to avoid cargo running
    // into a deadlock reading the workspace files.
    cmd.arg("--target-dir").arg(target_dir.as_os_str());

    let status = cmd.status().unwrap();
    if !status.success() {
        panic!(
            "Failed to build linux x86 setup header:\n\tcommand `{:?}`\n\treturned {}",
            cmd, status
        );
    }
}

fn encode_kernel_to_file(aster_elf: &AsterBin, encoding: PayloadEncoding) -> PathBuf {
    let kernel_path = aster_elf.path();
    let encoded_path = {
        let mut filename = kernel_path.file_name().unwrap().to_os_string();
        filename.push(".compressed");
        kernel_path.with_file_name(filename)
    };

    let mut kernel = Vec::new();
    File::open(kernel_path)
        .unwrap()
        .read_to_end(&mut kernel)
        .unwrap();

    let encoded = encode_kernel(kernel, encoding);
    File::create(&encoded_path)
        .unwrap()
        .write_all(&encoded)
        .unwrap();

    encoded_path
}
