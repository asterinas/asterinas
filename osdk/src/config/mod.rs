// SPDX-License-Identifier: MPL-2.0

//! This module is responsible for parsing configuration files and combining them with command-line parameters
//! to obtain the final configuration, it will also try searching system to fill valid values for specific
//! arguments if the arguments is missing, e.g., the path of QEMU. The final configuration is stored in `BuildConfig`,
//! `RunConfig` and `TestConfig`. These `*Config` are used for `build`, `run` and `test` subcommand.

mod eval;

pub mod manifest;
pub mod scheme;
pub mod unix_args;

#[cfg(test)]
mod test;

use std::{env, path::PathBuf};

use scheme::{Action, ActionScheme, BootScheme, Build, GrubScheme, QemuScheme, Scheme};

use crate::{
    arch::{get_default_arch, Arch},
    cli::CommonArgs,
    config::unix_args::apply_kv_array,
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

fn apply_args_before_finalize(action_scheme: &mut ActionScheme, args: &CommonArgs) {
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
            for seperated_arg in init_arg.split(' ') {
                boot.init_args.push(seperated_arg.to_string());
            }
        }
        if let Some(initramfs) = &args.initramfs {
            boot.initramfs = Some(initramfs.clone());
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
            qemu.path = Some(path.clone());
        }
        if let Some(bootdev_options) = &args.bootdev_append_options {
            qemu.bootdev_append_options = Some(bootdev_options.clone());
        }
    }
}

fn apply_args_after_finalize(action: &mut Action, args: &CommonArgs) {
    action.build.apply_common_args(args);
    action.qemu.apply_qemu_args(&args.qemu_args);
    if args.display_grub_menu {
        action.grub.display_grub_menu = true;
    }
}

impl Config {
    pub fn new(scheme: &Scheme, common_args: &CommonArgs) -> Self {
        let target_arch = common_args.target_arch.unwrap_or(get_default_arch());
        let default_scheme = ActionScheme {
            boot: scheme.boot.clone(),
            grub: scheme.grub.clone(),
            qemu: scheme.qemu.clone(),
            build: scheme.build.clone(),
        };
        let run = {
            let mut run = scheme.run.clone().unwrap_or_default();
            run.inherit(&default_scheme);
            apply_args_before_finalize(&mut run, common_args);
            let mut run = run.finalize(target_arch);
            apply_args_after_finalize(&mut run, common_args);
            run
        };
        let test = {
            let mut test = scheme.test.clone().unwrap_or_default();
            test.inherit(&default_scheme);
            apply_args_before_finalize(&mut test, common_args);
            let mut test = test.finalize(target_arch);
            apply_args_after_finalize(&mut test, common_args);
            test
        };
        Self {
            work_dir: scheme
                .work_dir
                .clone()
                .unwrap_or_else(|| env::current_dir().unwrap()),
            target_arch,
            build: scheme.build.clone().unwrap_or_default().finalize(),
            run,
            test,
        }
    }
}
