// SPDX-License-Identifier: MPL-2.0

use std::{
    path::{Path, PathBuf},
    process,
};

use serde::Deserialize;

use super::{
    boot::Boot,
    qemu::{CfgQemu, Qemu},
};
use crate::config_manager::cfg::Cfg;
use crate::{error::Errno, error_msg};

/// The osdk manifest from configuration file and command line arguments.
#[derive(Debug, Clone)]
pub struct OsdkManifest {
    pub kcmd_args: Vec<String>,
    pub initramfs: Option<PathBuf>,
    pub boot: Boot,
    pub qemu: Qemu,
}

impl OsdkManifest {
    pub fn from_toml_manifest(
        toml_manifest: TomlManifest,
        arch: Option<String>,
        selection: Option<String>,
    ) -> Self {
        let TomlManifest {
            mut kcmd_args,
            mut init_args,
            initramfs,
            boot,
            qemu,
        } = toml_manifest;
        let CfgQemu { default, cfg_map } = qemu;

        let Some(cfg_map) = cfg_map else {
            return Self {
                kcmd_args,
                initramfs,
                boot,
                qemu: default,
            };
        };

        for cfg in cfg_map.keys() {
            const ALLOWED_KEYS: &[&str] = &["arch", "select"];
            if !cfg.check_allowed(ALLOWED_KEYS) {
                error_msg!("cfg {:#?} is not allowed to be used in `OSDK.toml`", cfg);
                process::exit(Errno::ParseMetadata as _);
            }
        }

        let mut qemu_args = None;

        let mut args_matches: Vec<_> = if arch.is_none() && selection.is_none() {
            vec![]
        } else {
            let mut need_cfg = Cfg::new();
            if let Some(arch) = arch {
                need_cfg.insert("arch".to_string(), arch);
            }
            if let Some(selection) = selection {
                need_cfg.insert("select".to_string(), selection);
            }
            cfg_map
                .into_iter()
                .filter_map(
                    |(cfg, args)| {
                        if need_cfg == cfg {
                            Some(args)
                        } else {
                            None
                        }
                    },
                )
                .collect()
        };

        if args_matches.len() > 1 {
            error_msg!("Multiple CFGs matched using the command line arguments");
            process::exit(Errno::ParseMetadata as _);
        } else if args_matches.len() == 1 {
            qemu_args = Some(args_matches.remove(0));
        } else if args_matches.is_empty() {
            qemu_args = Some(default);
        }

        check_args("kcmd_args", &kcmd_args);
        check_args("init_args", &init_args);

        kcmd_args.push("--".to_string());
        kcmd_args.append(&mut init_args);

        OsdkManifest {
            kcmd_args,
            initramfs,
            boot,
            qemu: qemu_args.unwrap(),
        }
    }

    pub fn check_canonicalize_all_paths(&mut self, manifest_file_dir: impl AsRef<Path>) {
        macro_rules! canonicalize_path {
            ($path:expr) => {{
                let path = if $path.is_relative() {
                    manifest_file_dir.as_ref().join($path)
                } else {
                    $path.clone()
                };
                path.canonicalize().unwrap_or_else(|_| {
                    error_msg!("File specified but not found: {:#?}", path);
                    process::exit(Errno::ParseMetadata as _);
                })
            }};
        }
        macro_rules! canonicalize_optional_path {
            ($path:expr) => {
                if let Some(path_inner) = &$path {
                    Some(canonicalize_path!(path_inner))
                } else {
                    None
                }
            };
        }
        self.initramfs = canonicalize_optional_path!(self.initramfs);
        self.boot.grub_mkrescue = canonicalize_optional_path!(self.boot.grub_mkrescue);
        self.boot.ovmf = canonicalize_optional_path!(self.boot.ovmf);
        self.qemu.path = canonicalize_optional_path!(self.qemu.path);
        for drive_file in &mut self.qemu.drive_files {
            drive_file.path = canonicalize_path!(&drive_file.path);
        }
    }
}

/// The osdk manifest from configuration file `OSDK.toml`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct TomlManifest {
    /// Command line arguments for guest kernel
    #[serde(default)]
    pub kcmd_args: Vec<String>,
    #[serde(default)]
    pub init_args: Vec<String>,
    /// The path of initramfs
    pub initramfs: Option<PathBuf>,
    #[serde(default)]
    pub boot: Boot,
    #[serde(default)]
    pub qemu: CfgQemu,
}

fn check_args(arg_name: &str, args: &[String]) {
    for arg in args {
        if arg.as_str() == "--" {
            error_msg!("`{}` cannot have `--` as argument", arg_name);
            process::exit(Errno::ParseMetadata as _);
        }
    }
}
