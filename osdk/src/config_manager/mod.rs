// SPDX-License-Identifier: MPL-2.0

//! This module is responsible for parsing configuration files and combining them with command-line parameters
//! to obtain the final configuration, it will also try searching system to fill valid values for specific
//! arguments if the arguments is missing, e.g., the path of QEMU. The final configuration is stored in `BuildConfig`,
//! `RunConfig` and `TestConfig`. These `*Config` are used for `build`, `run` and `test` subcommand.

pub mod action;
pub mod cfg;
pub mod manifest;
pub mod qemu;
pub mod unix_args;

use action::ActionSettings;

#[cfg(test)]
mod test;

use std::{fs, path::PathBuf, process};

use self::manifest::{OsdkManifest, TomlManifest};
use crate::{
    arch::{get_default_arch, Arch},
    cli::{BuildArgs, CargoArgs, DebugArgs, GdbServerArgs, OsdkArgs, RunArgs, TestArgs},
    error::Errno,
    error_msg,
    util::get_cargo_metadata,
};

/// Configurations for build subcommand
#[derive(Debug)]
pub struct BuildConfig {
    pub arch: Arch,
    pub settings: ActionSettings,
    pub cargo_args: CargoArgs,
}

impl BuildConfig {
    pub fn parse(args: &BuildArgs) -> Self {
        let arch = args.osdk_args.arch.unwrap_or_else(get_default_arch);
        let cargo_args = parse_cargo_args(&args.cargo_args);
        let mut manifest = load_osdk_manifest(&args.cargo_args, &args.osdk_args);
        if let Some(run) = manifest.run.as_mut() {
            run.apply_cli_args(&args.osdk_args);
        }
        Self {
            arch,
            settings: manifest.run.unwrap(),
            cargo_args,
        }
    }
}

/// Configurations for run subcommand
#[derive(Debug, Clone)]
pub struct RunConfig {
    pub arch: Arch,
    pub settings: ActionSettings,
    pub cargo_args: CargoArgs,
    pub gdb_server_args: GdbServerArgs,
}

impl RunConfig {
    pub fn parse(args: &RunArgs) -> Self {
        let arch = args.osdk_args.arch.unwrap_or_else(get_default_arch);
        let cargo_args = parse_cargo_args(&args.cargo_args);
        let mut manifest = load_osdk_manifest(&args.cargo_args, &args.osdk_args);
        if let Some(run) = manifest.run.as_mut() {
            run.apply_cli_args(&args.osdk_args);
        }
        Self {
            arch,
            settings: manifest.run.unwrap(),
            cargo_args,
            gdb_server_args: args.gdb_server_args.clone(),
        }
    }
}

#[derive(Debug)]
pub struct DebugConfig {
    pub cargo_args: CargoArgs,
    pub remote: String,
}

impl DebugConfig {
    pub fn parse(args: &DebugArgs) -> Self {
        Self {
            cargo_args: parse_cargo_args(&args.cargo_args),
            remote: args.remote.clone(),
        }
    }
}

/// Configurations for test subcommand
#[derive(Debug)]
pub struct TestConfig {
    pub arch: Arch,
    pub settings: ActionSettings,
    pub cargo_args: CargoArgs,
    pub test_name: Option<String>,
}

impl TestConfig {
    pub fn parse(args: &TestArgs) -> Self {
        let arch = args.osdk_args.arch.unwrap_or_else(get_default_arch);
        let cargo_args = parse_cargo_args(&args.cargo_args);
        let manifest = load_osdk_manifest(&args.cargo_args, &args.osdk_args);
        // Use run settings if test settings are not provided
        let mut test = if let Some(test) = manifest.test {
            test
        } else {
            manifest.run.unwrap()
        };
        test.apply_cli_args(&args.osdk_args);
        Self {
            arch,
            settings: test,
            cargo_args,
            test_name: args.test_name.clone(),
        }
    }
}

fn load_osdk_manifest(cargo_args: &CargoArgs, osdk_args: &OsdkArgs) -> OsdkManifest {
    let feature_strings = get_feature_strings(cargo_args);
    let cargo_metadata = get_cargo_metadata(None::<&str>, Some(&feature_strings)).unwrap();
    let workspace_root = PathBuf::from(
        cargo_metadata
            .get("workspace_root")
            .unwrap()
            .as_str()
            .unwrap(),
    );

    // Search for OSDK.toml in the current directory. If not, dive into the workspace root.
    let manifest_path = PathBuf::from("OSDK.toml");
    let (contents, manifest_path) = if let Ok(contents) = fs::read_to_string("OSDK.toml") {
        (contents, manifest_path)
    } else {
        let manifest_path = workspace_root.join("OSDK.toml");
        let Ok(contents) = fs::read_to_string(&manifest_path) else {
            error_msg!(
                "Cannot read file {}",
                manifest_path.to_string_lossy().to_string()
            );
            process::exit(Errno::GetMetadata as _);
        };
        (contents, manifest_path)
    };

    let toml_manifest: TomlManifest = toml::from_str(&contents).unwrap_or_else(|err| {
        let span = err.span().unwrap();
        let wider_span =
            (span.start as isize - 20).max(0) as usize..(span.end + 20).min(contents.len());
        error_msg!(
            "Cannot parse TOML file, {}. {}:{:?}:\n {}",
            err.message(),
            manifest_path.to_string_lossy().to_string(),
            span,
            &contents[wider_span],
        );
        process::exit(Errno::ParseMetadata as _);
    });
    let osdk_manifest = toml_manifest.get_osdk_manifest(
        workspace_root,
        osdk_args.arch.unwrap_or_else(get_default_arch),
        osdk_args.schema.as_ref().map(|s| s.to_string()),
    );
    osdk_manifest
}

/// Parse cargo args.
/// 1. Split `features` in `cargo_args` to ensure each string contains exactly one feature.
/// 2. Change `profile` to `release` if `--release` is set.
fn parse_cargo_args(cargo_args: &CargoArgs) -> CargoArgs {
    let mut features = Vec::new();

    for feature in cargo_args.features.iter() {
        for feature in feature.split(',') {
            if !feature.is_empty() {
                features.push(feature.to_string());
            }
        }
    }

    let profile = if cargo_args.release {
        "release".to_string()
    } else {
        cargo_args.profile.clone()
    };

    CargoArgs {
        profile,
        release: cargo_args.release,
        features,
    }
}

fn get_feature_strings(cargo_args: &CargoArgs) -> Vec<String> {
    cargo_args
        .features
        .iter()
        .map(|feature| format!("--features={}", feature))
        .collect()
}
