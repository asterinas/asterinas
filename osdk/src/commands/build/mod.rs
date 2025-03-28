// SPDX-License-Identifier: MPL-2.0

mod bin;
mod grub;
mod qcow2;

use std::{
    ffi::OsString,
    path::{Path, PathBuf},
    process,
    time::SystemTime,
};

use bin::make_elf_for_qemu;

use super::util::{cargo, profile_name_adapter, COMMON_CARGO_ARGS, DEFAULT_TARGET_RELPATH};
use crate::{
    arch::Arch,
    base_crate::{new_base_crate, BaseCrateType},
    bundle::{
        bin::{AsterBin, AsterBinType, AsterElfMeta},
        file::BundleFile,
        Bundle,
    },
    cli::BuildArgs,
    config::{
        scheme::{ActionChoice, BootMethod},
        Config,
    },
    error::Errno,
    error_msg,
    util::{
        get_cargo_metadata, get_current_crates, get_kernel_crate, get_target_directory, CrateInfo,
        DirGuard,
    },
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

    let target_info = get_kernel_crate();

    let bundle_path = osdk_output_directory.join(target_info.name.clone());

    let action = if build_args.for_test {
        ActionChoice::Test
    } else {
        ActionChoice::Run
    };

    let _bundle = create_base_and_cached_build(
        target_info,
        bundle_path,
        &osdk_output_directory,
        &cargo_target_directory,
        config,
        action,
        &[],
    );
}

pub fn create_base_and_cached_build(
    target_crate: CrateInfo,
    bundle_path: impl AsRef<Path>,
    osdk_output_directory: impl AsRef<Path>,
    cargo_target_directory: impl AsRef<Path>,
    config: &Config,
    action: ActionChoice,
    rustflags: &[&str],
) -> Bundle {
    let base_crate_path = new_base_crate(
        match action {
            ActionChoice::Run => BaseCrateType::Run,
            ActionChoice::Test => BaseCrateType::Test,
        },
        osdk_output_directory.as_ref().join(&target_crate.name),
        &target_crate.name,
        &target_crate.path,
        false,
    );
    let _dir_guard = DirGuard::change_dir(&base_crate_path);
    do_cached_build(
        &bundle_path,
        &osdk_output_directory,
        &cargo_target_directory,
        config,
        action,
        rustflags,
    )
}

fn get_reusable_existing_bundle(
    bundle_path: impl AsRef<Path>,
    config: &Config,
    action: ActionChoice,
) -> Option<Bundle> {
    let existing_bundle = Bundle::load(&bundle_path);
    let Some(existing_bundle) = existing_bundle else {
        info!("Building a new bundle: No cached bundle found or validation of the existing bundle failed");
        return None;
    };
    if let Err(e) = existing_bundle.can_run_with_config(config, action) {
        info!("Building a new bundle: {}", e);
        return None;
    }
    let workspace_root = {
        let meta = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
        PathBuf::from(meta.get("workspace_root").unwrap().as_str().unwrap())
    };
    if existing_bundle.last_modified_time() < get_last_modified_time(&workspace_root) {
        info!("Building a new bundle: workspace_root has been updated");
        return None;
    }
    Some(existing_bundle)
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
    let (build, boot) = match action {
        ActionChoice::Run => (&config.run.build, &config.run.boot),
        ActionChoice::Test => (&config.test.build, &config.test.boot),
    };

    let mut rustflags = rustflags.to_vec();
    rustflags.push(&build.rustflags);
    let aster_elf = build_kernel_elf(
        config.target_arch,
        &build.profile,
        &build.features[..],
        build.no_default_features,
        &build.override_configs[..],
        &cargo_target_directory,
        &rustflags,
    );

    // Check the existing bundle's reusability
    if let Some(existing_bundle) = get_reusable_existing_bundle(&bundle_path, config, action) {
        if aster_elf.modified_time() < &existing_bundle.last_modified_time() {
            info!("Reusing existing bundle: aster_elf is unchanged");
            return existing_bundle;
        }
    }

    // Build a new bundle
    info!("Building a new bundle");
    if bundle_path.as_ref().exists() {
        std::fs::remove_dir_all(&bundle_path).unwrap();
    }
    let mut bundle = Bundle::new(&bundle_path, config, action);

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

    let mut rustflags = Vec::from(rustflags);
    // Asterinas does not support PIC yet.
    rustflags.extend(vec![
        &rustc_linker_script_arg,
        "-C relocation-model=static",
        "-C relro-level=off",
        // Even if we disabled unwinding on panic, we need to specify this to show backtraces.
        "-C force-unwind-tables=yes",
        // This is to let rustc know that "cfg(ktest)" is our well-known configuration.
        // See the [Rust Blog](https://blog.rust-lang.org/2024/05/06/check-cfg.html) for details.
        "--check-cfg cfg(ktest)",
        // The red zone is a small area below the stack pointer for optimization, primarily in
        // user-space applications. This optimization can be problematic in the kernel, as the CPU
        // or exception handlers may overwrite kernel data in the red zone. Therefore, we disable
        // this optimization.
        "-C no-redzone=y",
    ]);

    if matches!(arch, Arch::X86_64) {
        // This is a workaround for <https://github.com/asterinas/asterinas/issues/839>.
        // It makes running on Intel CPUs after Ivy Bridge (2012) faster, but much slower
        // on older CPUs.
        // If we are on late AMD machines, use `+fsrm` here. Otherwise we should `+ermsb`.
        rustflags.push("-C target-feature=+fsrm");
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

    const CFLAGS: &str = "CFLAGS_x86_64-unknown-none";
    let mut env_cflags = std::env::var(CFLAGS).unwrap_or_default();
    env_cflags += " -fPIC";

    if features.contains(&"coverage".to_string()) {
        // This is a workaround for minicov <https://github.com/Amanieu/minicov/issues/29>,
        // makes coverage work on x86_64-unknown-none.
        env_cflags += " -D__linux__";
    }

    command.env(CFLAGS, env_cflags);

    info!("Building kernel ELF using command: {:#?}", command);
    info!("Building directory: {:?}", std::env::current_dir().unwrap());

    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Cargo build failed");
        process::exit(Errno::ExecuteCommand as _);
    }

    let aster_bin_path = cargo_target_directory
        .as_ref()
        .join(&target_os_string)
        .join(profile_name_adapter(profile))
        .join(get_current_crates().remove(0).name);

    AsterBin::new(
        aster_bin_path,
        arch,
        AsterBinType::Elf(AsterElfMeta {
            has_linux_header: false,
            has_pvh_header: false,
            has_multiboot_header: true,
            has_multiboot2_header: true,
        }),
        get_current_crates().remove(0).version,
        false,
    )
}

fn get_last_modified_time(path: impl AsRef<Path>) -> SystemTime {
    if path.as_ref().is_file() {
        return path.as_ref().metadata().unwrap().modified().unwrap();
    }
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
