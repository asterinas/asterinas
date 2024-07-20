// SPDX-License-Identifier: MPL-2.0

use std::{
    fs::OpenOptions,
    io::{Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    process::Command,
};

use linux_bzimage_builder::{
    legacy32_rust_target_json, make_bzimage, BzImageType, PayloadEncoding,
};

use crate::{
    arch::Arch,
    bundle::{
        bin::{AsterBin, AsterBinType, AsterBzImageMeta, AsterElfMeta},
        file::BundleFile,
    },
    util::get_current_crate_info,
};

pub fn make_install_bzimage(
    install_dir: impl AsRef<Path>,
    target_dir: impl AsRef<Path>,
    aster_elf: &AsterBin,
    linux_x86_legacy_boot: bool,
    encoding: PayloadEncoding,
) -> AsterBin {
    let target_name = get_current_crate_info().name;
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
                let arch = SetupInstallArch::Other(gen_target_json_path.canonicalize().unwrap());
                install_setup_with_arch(setup_install_dir, setup_target_dir, &arch);
            }
            BzImageType::Efi64 => {
                install_setup_with_arch(
                    setup_install_dir,
                    setup_target_dir,
                    &SetupInstallArch::X86_64,
                );
            }
        };
        setup_install_dir.join("bin").join("linux-bzimage-setup")
    };
    // Make the `bzImage`-compatible kernel image and place it in the boot directory.
    let install_path = install_dir.as_ref().join(target_name);
    info!("Building bzImage");
    println!("install_path: {:?}", install_path);
    make_bzimage(
        &install_path,
        image_type,
        aster_elf.path(),
        &setup_bin,
        encoding,
    );

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
        let status = Command::new("rust-strip")
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
        std::fs::copy(elf.path(), &result_elf_path).unwrap();
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

enum SetupInstallArch {
    X86_64,
    Other(PathBuf),
}

fn install_setup_with_arch(
    install_dir: impl AsRef<Path>,
    target_dir: impl AsRef<Path>,
    arch: &SetupInstallArch,
) {
    if !target_dir.as_ref().exists() {
        std::fs::create_dir_all(&target_dir).unwrap();
    }
    let target_dir = std::fs::canonicalize(target_dir).unwrap();

    let mut cmd = Command::new("cargo");
    cmd.env("RUSTFLAGS", "-Ccode-model=kernel -Crelocation-model=pie -Ctarget-feature=+crt-static -Zplt=yes -Zrelax-elf-relocations=yes -Crelro-level=full");
    cmd.arg("install").arg("linux-bzimage-setup");
    cmd.arg("--force");
    cmd.arg("--root").arg(install_dir.as_ref());
    if std::env::var("AUTO_TEST").is_ok() || std::env::var("OSDK_INTEGRATION_TEST").is_ok() {
        cmd.arg("--path")
            .arg("../../../ostd/libs/linux-bzimage/setup");
    }
    // Remember to upgrade this version if new version of linux-bzimage-setup is released.
    const LINUX_BZIMAGE_SETUP_VERSION: &str = "0.1.0";
    cmd.arg("--version").arg(LINUX_BZIMAGE_SETUP_VERSION);
    cmd.arg("--target").arg(match arch {
        SetupInstallArch::X86_64 => "x86_64-unknown-none",
        SetupInstallArch::Other(path) => path.to_str().unwrap(),
    });
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
