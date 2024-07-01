// SPDX-License-Identifier: MPL-2.0

use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader, Write},
    path::{Path, PathBuf},
    process::Command,
};

use crate::{error::Errno, error_msg};

use quote::ToTokens;

/// The version of OSTD on crates.io.
///
/// OSTD shares the same version with OSDK, so just use the version of OSDK here.
pub const OSTD_VERSION: &str = env!("CARGO_PKG_VERSION");
pub fn ostd_dep() -> String {
    format!("ostd = {{ version = \"{}\" }}", OSTD_VERSION)
}

fn cargo() -> Command {
    Command::new("cargo")
}

/// Create a new library crate with cargo
pub fn cargo_new_lib(crate_name: &str) {
    let mut command = cargo();
    command.args(["new", "--lib", crate_name]);
    let status = command.status().unwrap();
    if !status.success() {
        error_msg!("Failed to create new crate");
        std::process::exit(Errno::CreateCrate as _);
    }
}

/// Get the Cargo metadata parsed from the standard output
/// of the invocation of Cargo. Return `None` if the command
/// fails or the `current_dir` is not in a Cargo workspace.
pub fn get_cargo_metadata<S1: AsRef<Path>, S2: AsRef<OsStr>>(
    current_dir: Option<S1>,
    cargo_args: Option<&[S2]>,
) -> Option<serde_json::Value> {
    let mut command = cargo();
    command.args(["metadata", "--no-deps", "--format-version", "1"]);

    if let Some(current_dir) = current_dir {
        command.current_dir(current_dir);
    }

    if let Some(cargo_args) = cargo_args {
        command.args(cargo_args);
    }

    let output = command.output().unwrap();

    if !output.status.success() {
        return None;
    }

    let stdout = String::from_utf8_lossy(&output.stdout);
    Some(serde_json::from_str(&stdout).unwrap())
}

pub fn get_target_directory() -> PathBuf {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
    metadata
        .get("target_directory")
        .unwrap()
        .as_str()
        .unwrap()
        .into()
}

pub struct CrateInfo {
    pub name: String,
    pub version: String,
    pub path: String,
}

/// Retrieve the default member in the workspace.
///
/// If there is only one kernel crate, return that crate;
/// If there are multiple kernel crates or no kernel crates in the workspace,
/// this function will exit with an error.
///
/// A crate is considered a kernel crate if it utilizes the `ostd::main` macro.
fn get_default_member(metadata: &serde_json::Value) -> &str {
    let default_members = metadata
        .get("workspace_default_members")
        .unwrap()
        .as_array()
        .unwrap();

    if default_members.len() == 1 {
        return default_members[0].as_str().unwrap();
    }

    let packages: Vec<_> = {
        let packages = metadata.get("packages").unwrap().as_array().unwrap();

        packages
            .iter()
            .filter(|package| {
                let id = package.get("id").unwrap();
                if !default_members.contains(id) {
                    return false;
                }

                let src_path = {
                    let targets = package.get("targets").unwrap().as_array().unwrap();
                    if targets.len() != 1 {
                        return false;
                    }
                    targets[0].get("src_path").unwrap().as_str().unwrap()
                };

                let file = {
                    let content = fs::read_to_string(src_path).unwrap();
                    syn::parse_file(&content).unwrap()
                };

                contains_ostd_main_macro(&file)
            })
            .collect()
    };

    if packages.is_empty() {
        error_msg!("OSDK requires there's at least one kernel package. Please navigate to the kernel package directory or the workspace root and run the command.");
        std::process::exit(Errno::BuildCrate as _);
    }

    if packages.len() >= 2 {
        error_msg!("OSDK requires there's at most one kernel package in the workspace. Please navigate to the kernel package directory and run the command.");
        std::process::exit(Errno::BuildCrate as _);
    }

    packages[0].get("id").unwrap().as_str().unwrap()
}

fn contains_ostd_main_macro(file: &syn::File) -> bool {
    for item in &file.items {
        let syn::Item::Fn(item_fn) = item else {
            continue;
        };

        for attr in &item_fn.attrs {
            let attr = format!("{}", attr.to_token_stream());
            if attr.as_str() == "# [ostd :: main]" || attr.as_str() == "#[main]" {
                return true;
            }
        }
    }

    false
}

pub fn get_current_crate_info() -> CrateInfo {
    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();

    let default_member = get_default_member(&metadata);
    parse_package_id_string(default_member)
}

pub fn parse_package_id_string(package_id: &str) -> CrateInfo {
    // Prior to 202403 (Rust 1.77.1), the package id string here is in the form of
    // "<crate_name> <crate_version> (path+file://<crate_path>)".
    // After that, it's
    // "path+file://<crate_path>#<crate_name>@<crate_version>", in which the crate
    // name might not exist if it is the last component of the path.
    if package_id.starts_with("path+file://") {
        // After 1.77.1
        if package_id.contains('@') {
            let package_id_segments = package_id.split(['#', '@']).collect::<Vec<&str>>();
            CrateInfo {
                name: package_id_segments[1].to_string(),
                version: package_id_segments[2].to_string(),
                path: package_id_segments[0]
                    .trim_start_matches("path+file://")
                    .to_string(),
            }
        } else {
            let package_id_segments = package_id.split(['#']).collect::<Vec<&str>>();
            let path = package_id_segments[0]
                .trim_start_matches("path+file://")
                .to_string();
            CrateInfo {
                name: PathBuf::from(path.clone())
                    .file_name()
                    .unwrap()
                    .to_str()
                    .unwrap()
                    .to_string(),
                version: package_id_segments[1].to_string(),
                path,
            }
        }
    } else {
        // Before 1.77.1
        let default_member = package_id.split(' ').collect::<Vec<&str>>();
        CrateInfo {
            name: default_member[0].to_string(),
            version: default_member[1].to_string(),
            path: default_member[2]
                .trim_start_matches("(path+file://")
                .trim_end_matches(')')
                .to_string(),
        }
    }
}

/// Print source line stack trace if a panic is detected from QEMU log.
///
/// The source line is produced with the `addr2line` command using the PC values in the panic
/// stack trace.
pub fn trace_panic_from_log(qemu_log: File, bin_path: PathBuf) {
    // We read last 500 lines since more than 100 layers of stack trace is unlikely.
    let reader = rev_buf_reader::RevBufReader::new(qemu_log);
    let lines: Vec<String> = reader.lines().take(500).map(|l| l.unwrap()).collect();
    let mut trace_exists = false;
    let mut stack_num = 0;
    let pc_matcher = regex::Regex::new(r" - pc (0x[0-9a-fA-F]+)").unwrap();
    let exe = bin_path.to_string_lossy();
    let mut addr2line = Command::new("addr2line");
    addr2line.args(["-e", &exe]);
    let mut addr2line_proc = addr2line
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();
    for line in lines.into_iter().rev() {
        if line.contains("printing stack trace:") {
            println!("[OSDK] The kernel seems panicked. Parsing stack trace for source lines:");
            trace_exists = true;
        }
        if trace_exists {
            if let Some(cap) = pc_matcher.captures(&line) {
                let pc = cap.get(1).unwrap().as_str();
                let mut stdin = addr2line_proc.stdin.as_ref().unwrap();
                stdin.write_all(pc.as_bytes()).unwrap();
                stdin.write_all(b"\n").unwrap();
                let mut line = String::new();
                let mut stdout = BufReader::new(addr2line_proc.stdout.as_mut().unwrap());
                stdout.read_line(&mut line).unwrap();
                stack_num += 1;
                println!("({: >3}) {}", stack_num, line.trim());
            }
        }
    }
}
