// SPDX-License-Identifier: MPL-2.0

//! This module is responsible for parsing configuration files and combining them with command-line parameters
//! to obtain the final configuration, it will also try searching system to fill valid values for specific
//! arguments if the arguments is missing, e.g., the path of QEMU. The final configuration is stored in `BuildConfig`,
//! `RunConfig` and `TestConfig`. These `*Config` are used for `build`, `run` and `test` subcommand.

pub mod manifest;
pub mod scheme;
pub mod unix_args;

#[cfg(test)]
mod test;

use std::{
    env, io,
    path::{Path, PathBuf},
    process,
};

use linux_bzimage_builder::PayloadEncoding;
use scheme::{
    Action, ActionScheme, BootProtocol, BootScheme, Build, GrubScheme, QemuScheme, Scheme,
};

use crate::{
    arch::{get_default_arch, Arch},
    cli::CommonArgs,
    config::unix_args::apply_kv_array,
    error::Errno,
    error_msg,
    util::new_command_checked_exists,
};

/// The global configuration for the OSDK actions.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Config {
    pub work_dir: PathBuf,
    pub target_arch: Arch,
    pub build: Build,
    pub run: Action,
    pub test: Action,
}

fn apply_args_before_finalize(
    action_scheme: &mut ActionScheme,
    args: &CommonArgs,
    workdir: &PathBuf,
) {
    if action_scheme.grub.is_none() {
        action_scheme.grub = Some(GrubScheme::default());
    }
    if let Some(ref mut grub) = action_scheme.grub {
        if let Some(grub_mkrescue) = &args.grub_mkrescue {
            grub.grub_mkrescue = Some(grub_mkrescue.clone());
        }
        if let Some(grub_boot_protocol) = args.grub_boot_protocol {
            grub.boot_protocol = Some(grub_boot_protocol);
        }
    }

    if action_scheme.boot.is_none() {
        action_scheme.boot = Some(BootScheme::default());
    }
    if let Some(ref mut boot) = action_scheme.boot {
        apply_kv_array(&mut boot.kcmd_args, &args.kcmd_args, "=", &[]);
        for init_arg in &args.init_args {
            for separated_arg in init_arg.split(' ') {
                boot.init_args.push(separated_arg.to_string());
            }
        }
        if let Some(initramfs) = &args.initramfs {
            let Ok(initramfs) = initramfs.canonicalize() else {
                error_msg!("The initramfs path provided with argument `--initramfs` does not match any files.");
                process::exit(Errno::GetMetadata as _);
            };
            boot.initramfs = Some(initramfs);
        }
        if let Some(boot_method) = args.boot_method {
            boot.method = Some(boot_method);
        }
    }

    if action_scheme.qemu.is_none() {
        action_scheme.qemu = Some(QemuScheme::default());
    }
    if let Some(ref mut qemu) = action_scheme.qemu {
        if let Some(path) = &args.qemu_exe {
            let Ok(qemu_path) = path.canonicalize() else {
                error_msg!(
                    "The QEMU path provided with argument `--qemu-exe` does not match any files."
                );
                process::exit(Errno::GetMetadata as _);
            };
            qemu.path = Some(qemu_path);
        }
        if let Some(bootdev_options) = &args.bootdev_append_options {
            qemu.bootdev_append_options = Some(bootdev_options.clone());
        }
    }

    canonicalize_and_eval(action_scheme, workdir);
}

fn canonicalize_and_eval(action_scheme: &mut ActionScheme, workdir: &PathBuf) {
    let canonicalize = |target: &mut PathBuf| {
        let last_cwd = std::env::current_dir().unwrap();
        std::env::set_current_dir(workdir).unwrap();

        *target = target.canonicalize().unwrap_or_else(|err| {
            error_msg!(
                "Cannot canonicalize path `{}`: {}",
                target.to_string_lossy(),
                err,
            );
            std::env::set_current_dir(&last_cwd).unwrap();
            process::exit(Errno::GetMetadata as _);
        });
        std::env::set_current_dir(last_cwd).unwrap();
    };

    if let Some(ref mut boot) = action_scheme.boot {
        if let Some(ref mut initramfs) = boot.initramfs {
            canonicalize(initramfs);
        }

        if let Some(ref mut qemu) = action_scheme.qemu {
            if let Some(ref mut qemu_path) = qemu.path {
                canonicalize(qemu_path);
            }
        }

        if let Some(ref mut grub) = action_scheme.grub {
            if let Some(ref mut grub_mkrescue_path) = grub.grub_mkrescue {
                canonicalize(grub_mkrescue_path);
            }
        }
    }

    // Do evaluations on the need to be evaluated string field, namely,
    // QEMU arguments.

    if let Some(ref mut qemu) = action_scheme.qemu {
        if let Some(ref mut args) = qemu.args {
            *args = match eval(workdir, args) {
                Ok(v) => v,
                Err(e) => {
                    error_msg!("Failed to evaluate qemu args: {:#?}", e);
                    process::exit(Errno::ParseMetadata as _);
                }
            }
        }
    }
}

/// This function is used to evaluate the string using the host's shell recursively
/// in order.
pub fn eval(cwd: impl AsRef<Path>, s: &String) -> io::Result<String> {
    let mut eval = new_command_checked_exists("bash");
    eval.arg("-c");
    eval.arg(format!("echo \"{}\"", s));
    eval.current_dir(cwd.as_ref());
    let output = eval.output()?;
    if !output.stderr.is_empty() {
        println!(
            "[Info] {}",
            String::from_utf8_lossy(&output.stderr).trim_end_matches('\n')
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string())
}

fn apply_args_after_finalize(action: &mut Action, args: &CommonArgs) {
    action.build.apply_common_args(args);
    action.qemu.apply_qemu_args(&args.qemu_args);
    if args.display_grub_menu {
        action.grub.display_grub_menu = true;
    }
    if args.coverage {
        action.qemu.args += " --no-shutdown";
        action.qemu.with_monitor = true;
    }
}

impl Config {
    pub fn new(scheme: &Scheme, common_args: &CommonArgs) -> Self {
        let check_compatibility = |protocol: BootProtocol, encoding: PayloadEncoding| {
            if protocol != BootProtocol::Linux && encoding != PayloadEncoding::Raw {
                panic!("The encoding format is not allowed to be specified if the boot protocol is not {:#?}", BootProtocol::Linux);
            }
        };
        let target_arch = common_args.target_arch.unwrap_or(get_default_arch());
        let default_scheme = ActionScheme {
            boot: scheme.boot.clone(),
            grub: scheme.grub.clone(),
            qemu: scheme.qemu.clone(),
            build: scheme.build.clone(),
        };
        let build = {
            let mut build = scheme.build.clone().unwrap_or_default().finalize();
            build.apply_common_args(common_args);
            build
        };
        let run = {
            let mut run = scheme.run.clone().unwrap_or_default();
            run.inherit(&default_scheme);
            apply_args_before_finalize(&mut run, common_args, scheme.work_dir.as_ref().unwrap());
            let mut run = run.finalize(target_arch);
            apply_args_after_finalize(&mut run, common_args);
            check_compatibility(run.grub.boot_protocol, run.build.encoding.clone());
            run
        };
        let test = {
            let mut test = scheme.test.clone().unwrap_or_default();
            test.inherit(&default_scheme);
            apply_args_before_finalize(&mut test, common_args, scheme.work_dir.as_ref().unwrap());
            let mut test = test.finalize(target_arch);
            apply_args_after_finalize(&mut test, common_args);
            check_compatibility(test.grub.boot_protocol, test.build.encoding.clone());
            test
        };
        Self {
            work_dir: scheme
                .work_dir
                .clone()
                .unwrap_or_else(|| env::current_dir().unwrap()),
            target_arch,
            build,
            run,
            test,
        }
    }
}
