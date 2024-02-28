// SPDX-License-Identifier: MPL-2.0

use std::{
    path::{Path, PathBuf},
    process,
};

use regex::Regex;
use serde::Deserialize;

use super::{
    boot::Boot,
    qemu::{CfgQemu, Qemu},
};
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
    pub fn from_toml_manifest<S: AsRef<str>>(
        toml_manifest: TomlManifest,
        selection: Option<S>,
    ) -> Self {
        let TomlManifest {
            mut kcmd_args,
            mut init_args,
            initramfs,
            boot,
            qemu,
        } = toml_manifest;
        let CfgQemu { default, cfg } = qemu;

        let Some(cfg) = cfg else {
            return Self {
                kcmd_args,
                initramfs,
                boot,
                qemu: default,
            };
        };

        for cfg in cfg.keys() {
            check_cfg(cfg);
        }

        let mut qemu_args = None;

        let mut selected_args: Vec<_> = if let Some(sel) = selection {
            cfg.into_iter()
                .filter_map(|(cfg, args)| {
                    if cfg.contains(sel.as_ref()) {
                        Some(args)
                    } else {
                        None
                    }
                })
                .collect()
        } else {
            vec![]
        };

        if selected_args.len() > 1 {
            error_msg!("Multiple selections are not allowed");
            process::exit(Errno::ParseMetadata as _);
        } else if selected_args.len() == 1 {
            qemu_args = Some(selected_args.remove(0));
        } else if selected_args.is_empty() {
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

/// Check cfg that is in the form that we can accept
fn check_cfg(cfg: &str) {
    if SELECT_REGEX.captures(cfg).is_none() {
        error_msg!("{} is not allowed to be used after `qemu` in `OSDK.toml`. Currently we only allow cfgs like `cfg(select=\"foo\")`", cfg);
        process::exit(Errno::ParseMetadata as _);
    }
}

lazy_static::lazy_static! {
    pub static ref SELECT_REGEX: Regex = Regex::new(r#"cfg\(select="(?P<select>\w+)"\)"#).unwrap();
}

#[cfg(test)]
mod test {
    use super::*;

    #[test]
    fn extract_selection() {
        let text = "cfg(select=\"abc123_\")";
        let captures = SELECT_REGEX.captures(text).unwrap();
        let selection = captures.name("select").unwrap().as_str();
        assert_eq!(selection, "abc123_");
    }
}
