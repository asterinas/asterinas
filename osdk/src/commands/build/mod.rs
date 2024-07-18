// SPDX-License-Identifier: MPL-2.0

mod bin;
mod grub;
mod qcow2;

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process,
    time::{Duration, SystemTime},
};

use bin::make_elf_for_qemu;

use super::util::{cargo, profile_name_adapter, COMMON_CARGO_ARGS, DEFAULT_TARGET_RELPATH};
use crate::{
    arch::Arch,
    base_crate::new_base_crate,
    bundle::{
        bin::{AsterBin, AsterBinType, AsterElfMeta},
        Bundle,
    },
    cli::BuildArgs,
    config::{
        scheme::{ActionChoice, BootMethod},
        Config,
    },
    error::Errno,
    error_msg,
    util::{get_cargo_metadata, get_current_crate_info, get_target_directory},
};

pub fn execute_build_command(config: &Config, build_args: &BuildArgs) {
    let cargo_target_directory = get_target_directory();
    let osdk_output_directory = build_args
        .output
        .clone()
        .unwrap_or(cargo_target_directory.join(DEFAULT_TARGET_RELPATH));
    if !osdk_output_directory.exists() {
        std::fs::create_dir_all(&osdk_output_directory).unwrap();
    }
    let target_info = get_current_crate_info();
    let bundle_path = osdk_output_directory.join(target_info.name);

    let action = if build_args.for_test {
        ActionChoice::Test
    } else {
        ActionChoice::Run
    };

    let _bundle = create_base_and_cached_build(
        bundle_path,
        &osdk_output_directory,
        &cargo_target_directory,
        config,
        action,
        &[],
    );
}

pub fn create_base_and_cached_build(
    bundle_path: impl AsRef<Path>,
    osdk_output_directory: impl AsRef<Path>,
    cargo_target_directory: impl AsRef<Path>,
    config: &Config,
    action: ActionChoice,
    rustflags: &[&str],
) -> Bundle {
    let base_crate_path = osdk_output_directory.as_ref().join("base");
    new_base_crate(
        &base_crate_path,
        &get_current_crate_info().name,
        get_current_crate_info().path,
    );
    let original_dir = std::env::current_dir().unwrap();
    std::env::set_current_dir(&base_crate_path).unwrap();
    let bundle = do_cached_build(
        &bundle_path,
        &osdk_output_directory,
        &cargo_target_directory,
        config,
        action,
        rustflags,
    );
    std::env::set_current_dir(original_dir).unwrap();
    bundle
}

/// If the source is not since modified and the last build is recent, we can reuse the existing bundle.
pub fn do_cached_build(
    bundle_path: impl AsRef<Path>,
    osdk_output_directory: impl AsRef<Path>,
    cargo_target_directory: impl AsRef<Path>,
    config: &Config,
    action: ActionChoice,
    rustflags: &[&str],
) -> Bundle {
    let build_a_new_one = || {
        do_build(
            &bundle_path,
            &osdk_output_directory,
            &cargo_target_directory,
            config,
            action,
            rustflags,
        )
    };

    let existing_bundle = Bundle::load(&bundle_path);
    let Some(existing_bundle) = existing_bundle else {
        return build_a_new_one();
    };
    if existing_bundle.can_run_with_config(config, action).is_err() {
        return build_a_new_one();
    }
    let Ok(built_since) = SystemTime::now().duration_since(existing_bundle.last_modified_time())
    else {
        return build_a_new_one();
    };
    if built_since > Duration::from_secs(600) {
        return build_a_new_one();
    }
    let workspace_root = {
        let meta = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
        PathBuf::from(meta.get("workspace_root").unwrap().as_str().unwrap())
    };
    if get_last_modified_time(workspace_root) < existing_bundle.last_modified_time() {
        return existing_bundle;
    }
    build_a_new_one()
}

pub fn do_build(
    bundle_path: impl AsRef<Path>,
    osdk_output_directory: impl AsRef<Path>,
    cargo_target_directory: impl AsRef<Path>,
    config: &Config,
    action: ActionChoice,
    rustflags: &[&str],
) -> Bundle {
    if bundle_path.as_ref().exists() {
        std::fs::remove_dir_all(&bundle_path).unwrap();
    }
    let mut bundle = Bundle::new(&bundle_path, config, action);

    let (build, boot) = match action {
        ActionChoice::Run => (&config.run.build, &config.run.boot),
        ActionChoice::Test => (&config.test.build, &config.test.boot),
    };

    let aster_elf = build_kernel_elf(
        config.target_arch,
        &build.profile,
        &build.features[..],
        build.no_default_features,
        &build.override_configs[..],
        &cargo_target_directory,
        rustflags,
    );

    match boot.method {
        BootMethod::GrubRescueIso | BootMethod::GrubQcow2 => {
            info!("Building boot device image");
            let bootdev_image = grub::create_bootdev_image(
                &osdk_output_directory,
                &aster_elf,
                boot.initramfs.as_ref(),
                config,
                action,
            );
            if matches!(boot.method, BootMethod::GrubQcow2) {
                let qcow2_image = qcow2::convert_iso_to_qcow2(bootdev_image);
                bundle.consume_vm_image(qcow2_image);
            } else {
                bundle.consume_vm_image(bootdev_image);
            }
            bundle.consume_aster_bin(aster_elf);
        }
        BootMethod::QemuDirect => {
            let qemu_elf = make_elf_for_qemu(&osdk_output_directory, &aster_elf, build.strip_elf);
            bundle.consume_aster_bin(qemu_elf);
        }
    }

    bundle
}

fn build_kernel_elf(
    arch: Arch,
    profile: &str,
    features: &[String],
    no_default_features: bool,
    override_configs: &[String],
    cargo_target_directory: impl AsRef<Path>,
    rustflags: &[&str],
) -> AsterBin {
    let target_os_string = OsString::from(&arch.triple());
    let rustc_linker_script_arg = format!("-C link-arg=-T{}.ld", arch);

    let env_rustflags = std::env::var("RUSTFLAGS").unwrap_or_default();
    let mut rustflags = Vec::from(rustflags);
    // Asterinas does not support PIC yet.
    rustflags.extend(vec![
        &env_rustflags,
        &rustc_linker_script_arg,
        "-C relocation-model=static",
        "-C relro-level=off",
        // We do not really allow unwinding except for kernel testing. However, we need to specify
        // this to show backtraces when panicking.
        "-C panic=unwind",
        // This is to let rustc know that "cfg(ktest)" is our well-known configuration.
        // See the [Rust Blog](https://blog.rust-lang.org/2024/05/06/check-cfg.html) for details.
        "--check-cfg cfg(ktest)",
    ]);

    if matches!(arch, Arch::X86_64) {
        // This is a workaround for <https://github.com/asterinas/asterinas/issues/839>.
        // It makes running on Intel CPUs after Ivy Bridge (2012) faster, but much slower
        // on older CPUs.
        rustflags.push("-C target-feature=+ermsb");
    }

    let mut command = cargo();
    command.env_remove("RUSTUP_TOOLCHAIN");
    command.env("RUSTFLAGS", rustflags.join(" "));
    command.arg("build");
    command.arg("--features").arg(features.join(" "));
    if no_default_features {
        command.arg("--no-default-features");
    }
    command.arg("--target").arg(&target_os_string);
    command
        .arg("--target-dir")
        .arg(cargo_target_directory.as_ref());
    command.args(COMMON_CARGO_ARGS);
    command.arg("--profile=".to_string() + profile);
    for override_config in override_configs {
        command.arg("--config").arg(override_config);
    }

    info!("Building kernel ELF using command: {:#?}", command);

    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Cargo build failed");
        process::exit(Errno::ExecuteCommand as _);
    }

    let aster_bin_path = cargo_target_directory
        .as_ref()
        .join(&target_os_string)
        .join(profile_name_adapter(profile))
        .join(get_current_crate_info().name);

    AsterBin::new(
        aster_bin_path,
        arch,
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

fn get_last_modified_time(path: impl AsRef<Path>) -> SystemTime {
    let mut last_modified = SystemTime::UNIX_EPOCH;
    for entry in std::fs::read_dir(path).unwrap() {
        let entry = entry.unwrap();
        if entry.file_name() == "target" {
            continue;
        }

        let metadata = entry.metadata().unwrap();
        if metadata.is_dir() {
            last_modified = std::cmp::max(last_modified, get_last_modified_time(entry.path()));
        } else {
            last_modified = std::cmp::max(last_modified, metadata.modified().unwrap());
        }
    }
    last_modified
}
