// SPDX-License-Identifier: MPL-2.0

use std::{
    collections::HashMap,
    ffi::OsStr,
    fs::{self, File},
    io::{BufRead, BufReader, Result, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::Command,
    sync::{LazyLock, Mutex},
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
    new_command_checked_exists("cargo")
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

/// Information about an OSDK crate.
#[derive(Debug, Clone)]
pub struct CrateInfo {
    pub name: String,
    pub version: String,
    pub path: String,
    /// Whether the crate is a kernel crate.
    ///
    /// If a crate contains the `ostd::main` macro, it is a kernel crate (akin
    /// to the binary crate in a normal Rust project, but it is a library from
    /// the perspective of Cargo).
    pub is_kernel_crate: bool,
}

/// Gets the default crates in the current directory.
pub fn get_current_crates() -> Vec<CrateInfo> {
    // To avoid the overhead of parsing the Cargo metadata and reading the
    // source multiple times.
    static MEMOIZED: LazyLock<Mutex<HashMap<PathBuf, Vec<CrateInfo>>>> =
        LazyLock::new(|| Mutex::new(HashMap::new()));

    let current_dir = std::env::current_dir().unwrap();
    let mut memoized = MEMOIZED.lock().unwrap();
    if let Some(crates) = memoized.get(&current_dir) {
        return crates.clone();
    }

    // If the current directory's crate info is not memoized, parse the Cargo
    // metadata.

    let metadata = get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap();
    let packages = metadata.get("packages").unwrap().as_array().unwrap();

    let default_members = metadata
        .get("workspace_default_members")
        .unwrap()
        .as_array()
        .unwrap();

    let mut result = Vec::new();

    for member in default_members {
        let member_meta = packages
            .iter()
            .find(|p| {
                let Some(p_id) = p.get("id") else {
                    return false;
                };
                p_id == member
            })
            .unwrap();

        let is_kernel_crate = package_contains_ostd_main(member_meta);

        let parsed_id = parse_package_id_string(member.as_str().unwrap());

        result.push(CrateInfo {
            name: parsed_id.name,
            version: parsed_id.version,
            path: parsed_id.path,
            is_kernel_crate,
        });
    }

    memoized.insert(current_dir, result.clone());

    result
}

/// Get the kernel crate in the current directory.
///
/// If the current directory is a virtual workspace and no/multiple kernel
/// crate is found, This function will print an error message and exit the
/// process.
pub fn get_kernel_crate() -> CrateInfo {
    let crates = get_current_crates();
    let kernel_crates: Vec<_> = crates.iter().filter(|c| c.is_kernel_crate).collect();
    if kernel_crates.len() == 1 {
        kernel_crates[0].clone()
    } else if kernel_crates.is_empty() {
        error_msg!("No kernel crate found in the current workspace");
        std::process::exit(Errno::NoKernelCrate as _);
    } else {
        error_msg!("Multiple kernel crates found in the current workspace");
        std::process::exit(Errno::TooManyCrates as _);
    }
}

/// Check if an OSDK dependent executable (e.g. QEMU) exists.
///
/// If it exists, create a command. If not, print an error message and exit the
/// process.
pub fn new_command_checked_exists(executable: impl AsRef<Path>) -> Command {
    let executable = executable.as_ref();
    if which::which(executable).is_err() {
        error_msg!(
            "Executable {:?} cannot be found in PATH, please install the corresponding package",
            executable
        );
        std::process::exit(Errno::ExecutableNotFound as _);
    }
    Command::new(executable)
}

fn package_contains_ostd_main(package: &serde_json::Value) -> bool {
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

    file_contains_ostd_main_macro(&file)
}

fn file_contains_ostd_main_macro(file: &syn::File) -> bool {
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

struct ParsedID {
    pub name: String,
    pub version: String,
    pub path: String,
}

fn parse_package_id_string(package_id: &str) -> ParsedID {
    // Prior to 202403 (Rust 1.77.1), the package id string here is in the form of
    // "<crate_name> <crate_version> (path+file://<crate_path>)".
    // After that, it's
    // "path+file://<crate_path>#<crate_name>@<crate_version>", in which the crate
    // name might not exist if it is the last component of the path.
    if package_id.starts_with("path+file://") {
        // After 1.77.1
        if package_id.contains('@') {
            let package_id_segments = package_id.split(['#', '@']).collect::<Vec<&str>>();
            ParsedID {
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
            ParsedID {
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
        ParsedID {
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
    let lines: Vec<String> = reader.lines().take(500).filter_map(|l| l.ok()).collect();
    let mut trace_exists = false;
    let mut stack_num = 0;
    let pc_matcher = regex::Regex::new(r" - pc (0x[0-9a-fA-F]+)").unwrap();
    let exe = bin_path.to_string_lossy();

    let mut addr2line = new_command_checked_exists("addr2line");
    addr2line.args(["-e", &exe]);
    let mut addr2line_proc = addr2line
        .stdin(std::process::Stdio::piped())
        .stdout(std::process::Stdio::piped())
        .spawn()
        .unwrap();

    for line in lines.into_iter().rev() {
        if line.contains("Printing stack trace:") {
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
    addr2line_proc.kill().unwrap();
    addr2line_proc.wait().unwrap();
}

/// Dump the coverage data from QEMU if the coverage information is found in the log.
pub fn dump_coverage_from_qemu(qemu_log: File, monitor_socket: &mut UnixStream) {
    const COVERAGE_SIGNATRUE: &str = "#### Coverage: ";
    let reader = rev_buf_reader::RevBufReader::new(qemu_log);

    let Some(line) = reader
        .lines()
        .find(|l| l.as_ref().unwrap().starts_with(COVERAGE_SIGNATRUE))
        .map(|l| l.unwrap())
    else {
        return;
    };

    let line = line.strip_prefix(COVERAGE_SIGNATRUE).unwrap();
    let (addr, size) = line.split_once(' ').unwrap();
    let addr = usize::from_str_radix(addr.strip_prefix("0x").unwrap(), 16).unwrap();
    let size: usize = size.parse().unwrap();

    let cmd = format!("memsave 0x{addr:x} {size} coverage.profraw\n");
    if monitor_socket.write_all(cmd.as_bytes()).is_ok() {
        info!("Coverage data saved to coverage.profraw");
    }
}

/// A guard that ensures the current working directory is restored
/// to its original state when the guard goes out of scope.
pub struct DirGuard(PathBuf);

impl DirGuard {
    /// Creates a new `DirGuard` that restores the provided directory
    /// when it goes out of scope.
    ///
    /// # Arguments
    ///
    /// * `original_dir` - The directory to restore when the guard is dropped.
    pub fn new(original_dir: PathBuf) -> Self {
        Self(original_dir)
    }

    /// Creates a new `DirGuard` using the current working directory as the original directory.
    pub fn from_current_dir() -> Self {
        Self::new(std::env::current_dir().unwrap())
    }

    /// Stores the current directory as the original directory and
    /// changes the working directory to the specified `new_dir`.
    ///
    /// # Arguments
    ///
    /// * `new_dir` - The directory to switch to.
    pub fn change_dir(new_dir: impl AsRef<Path>) -> Self {
        let original_dir_guard = DirGuard::from_current_dir();
        std::env::set_current_dir(new_dir.as_ref()).unwrap();
        original_dir_guard
    }
}

impl Drop for DirGuard {
    fn drop(&mut self) {
        std::env::set_current_dir(&self.0).unwrap();
    }
}

/// Attempts to create a hard link from `from` to `to`.
/// If the hard link operation fails (e.g., due to crossing file systems),
/// it falls back to performing a file copy.
///
/// # Arguments
/// - `from`: The source file path.
/// - `to`: The destination file path.
///
/// # Returns
/// - `Ok(0)` if the hard link is successfully created (no data was copied).
/// - `Ok(size)` where `size` is the number of bytes copied if the hard link failed and a copy was performed.
/// - `Err(error)` if an error occurred during the copy operation.
pub fn hard_link_or_copy<P: AsRef<Path>, Q: AsRef<Path>>(from: P, to: Q) -> Result<u64> {
    if fs::hard_link(&from, &to).is_err() {
        info!("Copying {:?} -> {:?}", from.as_ref(), to.as_ref());
        return fs::copy(from, to);
    }
    info!("Linking {:?} -> {:?}", from.as_ref(), to.as_ref());
    Ok(0)
}
