// SPDX-License-Identifier: MPL-2.0

mod bin;
mod grub;

use std::{path::Path, process};

use bin::strip_elf_for_qemu;

use super::util::{cargo, COMMON_CARGO_ARGS, DEFAULT_TARGET_RELPATH};
use crate::{
    base_crate::new_base_crate,
    bundle::{
        bin::{AsterBin, AsterBinType, AsterElfMeta},
        file::Initramfs,
        Bundle,
    },
    cli::CargoArgs,
    config_manager::{qemu::QemuMachine, BuildConfig},
    error::Errno,
    error_msg,
    util::{get_current_crate_info, get_target_directory},
};

pub fn execute_build_command(config: &BuildConfig) {
    let ws_target_directory = get_target_directory();
    let osdk_target_directory = ws_target_directory.join(DEFAULT_TARGET_RELPATH);
    if !osdk_target_directory.exists() {
        std::fs::create_dir_all(&osdk_target_directory).unwrap();
    }
    let target_info = get_current_crate_info();
    let bundle_path = osdk_target_directory.join(target_info.name);

    let _bundle = create_base_and_build(
        bundle_path,
        &osdk_target_directory,
        &ws_target_directory,
        config,
        &[],
    );
}

pub fn create_base_and_build(
    bundle_path: impl AsRef<Path>,
    osdk_target_directory: impl AsRef<Path>,
    cargo_target_directory: impl AsRef<Path>,
    config: &BuildConfig,
    rustflags: &[&str],
) -> Bundle {
    let base_crate_path = osdk_target_directory.as_ref().join("base");
    new_base_crate(
        &base_crate_path,
        &get_current_crate_info().name,
        get_current_crate_info().path,
    );
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&base_crate_path).unwrap();
    let bundle = do_build(
        &bundle_path,
        &osdk_target_directory,
        &cargo_target_directory,
        config,
        rustflags,
    );
    std::env::set_current_dir(original_dir).unwrap();
    bundle
}

pub fn do_build(
    bundle_path: impl AsRef<Path>,
    osdk_target_directory: impl AsRef<Path>,
    cargo_target_directory: impl AsRef<Path>,
    config: &BuildConfig,
    rustflags: &[&str],
) -> Bundle {
    if bundle_path.as_ref().exists() {
        std::fs::remove_dir_all(&bundle_path).unwrap();
    }
    let mut bundle = Bundle::new(
        &bundle_path,
        config.manifest.kcmd_args.clone(),
        config.manifest.boot.clone(),
        config.manifest.qemu.clone(),
        config.cargo_args.clone(),
    );

    if let Some(ref initramfs) = config.manifest.initramfs {
        if !initramfs.exists() {
            error_msg!("initramfs file not found: {}", initramfs.display());
            process::exit(Errno::BuildCrate as _);
        }
        bundle.add_initramfs(Initramfs::new(initramfs));
    };

    info!("Building kernel ELF");
    let aster_elf = build_kernel_elf(&config.cargo_args, &cargo_target_directory, rustflags);

    if matches!(config.manifest.qemu.machine, QemuMachine::Microvm) {
        let stripped_elf = strip_elf_for_qemu(&osdk_target_directory, &aster_elf);
        bundle.consume_aster_bin(stripped_elf);
    }

    // TODO: A boot device is required if we use GRUB. Actually you can boot
    // a multiboot kernel with Q35 machine directly without a bootloader.
    // We are currently ignoring this case.
    if matches!(config.manifest.qemu.machine, QemuMachine::Q35) {
        info!("Building boot device image");
        let bootdev_image = grub::create_bootdev_image(
            &osdk_target_directory,
            &aster_elf,
            config.manifest.initramfs.as_ref(),
            config,
        );
        bundle.consume_vm_image(bootdev_image);
    }

    bundle
}

fn build_kernel_elf(
    args: &CargoArgs,
    cargo_target_directory: impl AsRef<Path>,
    rustflags: &[&str],
) -> AsterBin {
    let target = "x86_64-unknown-none";

    let env_rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let mut rustflags = Vec::from(rustflags);
    // We disable RELRO and PIC here because they cause link failures
    rustflags.extend(vec![
        &env_rustflags,
        "-C link-arg=-Tx86_64.ld",
        "-C code-model=kernel",
        "-C relocation-model=static",
        "-Z relro-level=off",
    ]);

    let mut command = cargo();
    command.env_remove("RUSTUP_TOOLCHAIN");
    command.env("RUSTFLAGS", rustflags.join(" "));
    command.arg("build");
    command.arg("--target").arg(target);
    command
        .arg("--target-dir")
        .arg(cargo_target_directory.as_ref());
    command.args(COMMON_CARGO_ARGS);
    command.arg("--profile=".to_string() + &args.profile);
    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Cargo build failed");
        process::exit(Errno::ExecuteCommand as _);
    }

    let aster_bin_path = cargo_target_directory.as_ref().join(target);
    let aster_bin_path = if args.profile == "dev" {
        aster_bin_path.join("debug")
    } else {
        aster_bin_path.join(&args.profile)
    }
    .join(get_current_crate_info().name);

    AsterBin::new(
        aster_bin_path,
        AsterBinType::Elf(AsterElfMeta {
            has_linux_header: false,
            has_pvh_header: false,
            has_multiboot_header: true,
            has_multiboot2_header: true,
        }),
        get_current_crate_info().version,
        false,
    )
}
