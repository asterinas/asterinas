// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;
use std::process;

use regex::Regex;
use serde::Deserialize;

use crate::error::Errno;
use crate::error_msg;

use super::boot::Boot;
use super::qemu::{CfgQemu, Qemu};

/// The osdk manifest from configuration file and command line arguments.
#[derive(Debug)]
pub struct OsdkManifest {
    pub kcmd_args: Vec<String>,
    pub initramfs: Option<PathBuf>,
    pub boot: Boot,
    pub qemu: Qemu,
}

impl OsdkManifest {
    pub fn from_toml_manifest<S: AsRef<str>>(toml_manifest: TomlManifest, features: &[S]) -> Self {
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

        let mut feature_enabled_args: Vec<_> = cfg
            .into_iter()
            .filter_map(|(cfg, args)| {
                if features
                    .iter()
                    .any(|feature| cfg.contains(feature.as_ref()))
                {
                    Some(args)
                } else {
                    None
                }
            })
            .collect();

        if feature_enabled_args.len() > 1 {
            error_msg!("Multiple features are conflict");
            process::exit(Errno::ParseMetadata as _);
        } else if feature_enabled_args.len() == 1 {
            qemu_args = Some(feature_enabled_args.remove(0));
        } else if feature_enabled_args.is_empty() {
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
    if FEATURE_REGEX.captures(cfg).is_none() {
        error_msg!("{} is not allowed to used after `qemu` in `OSDK.toml`. Currently we only allowed cfg like `cfg(feature=\"foo\")`", cfg);
        process::exit(Errno::ParseMetadata as _);
    }
}

lazy_static::lazy_static! {
    pub static ref FEATURE_REGEX: Regex = Regex::new(r#"cfg\(feature="(?P<feature>\w+)"\)"#).unwrap();
}
