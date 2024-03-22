// SPDX-License-Identifier: MPL-2.0

use std::{path::PathBuf, process};

use clap::ValueEnum;

use super::{qemu, unix_args::apply_kv_array};

use crate::{config_manager::OsdkArgs, error::Errno, error_msg};

/// The settings for an action (running or testing).
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct ActionSettings {
    /// Command line arguments for the guest kernel
    #[serde(default)]
    pub kcmd_args: Vec<String>,
    /// Command line arguments for the guest init process
    #[serde(default)]
    pub init_args: Vec<String>,
    /// The path of initramfs
    pub initramfs: Option<PathBuf>,
    pub bootloader: Option<Bootloader>,
    pub boot_protocol: Option<BootProtocol>,
    /// The path of `grub_mkrecue`. Only be `Some(_)` if `loader` is `Bootloader::grub`
    pub grub_mkrescue: Option<PathBuf>,
    /// The path of OVMF binaries. Only required if `protocol` is `BootProtocol::LinuxEfiHandover64`
    pub ovmf: Option<PathBuf>,
    /// The path of OpenSBI binaries. Only required for RISC-V.
    pub opensbi: Option<PathBuf>,
    /// QEMU's available machines appended with various machine configurations
    pub qemu_machine: Option<String>,
    /// The additional arguments for running QEMU, except `-cpu` and `-machine`
    #[serde(default)]
    pub qemu_args: Vec<String>,
    /// The additional drive files attaching to QEMU
    #[serde(default)]
    pub drive_files: Vec<DriveFile>,
    /// The path of qemu
    pub qemu_exe: Option<PathBuf>,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum Bootloader {
    Grub,
    Qemu,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum BootProtocol {
    LinuxEfiHandover64,
    LinuxLegacy32,
    Multiboot,
    Multiboot2,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct DriveFile {
    pub path: PathBuf,
    pub append: String,
}

impl ActionSettings {
    pub fn canonicalize_paths(&mut self, cur_dir: impl AsRef<std::path::Path>) {
        macro_rules! canonicalize_path {
            ($path:expr) => {{
                let path = if $path.is_relative() {
                    cur_dir.as_ref().join($path)
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
        self.grub_mkrescue = canonicalize_optional_path!(self.grub_mkrescue);
        self.ovmf = canonicalize_optional_path!(self.ovmf);
        self.qemu_exe = canonicalize_optional_path!(self.qemu_exe);
        self.opensbi = canonicalize_optional_path!(self.opensbi);
        for drive_file in &mut self.drive_files {
            drive_file.path = canonicalize_path!(&drive_file.path);
        }
    }

    pub fn apply_cli_args(&mut self, args: &OsdkArgs) {
        macro_rules! apply {
            ($item:expr, $arg:expr) => {
                if let Some(arg) = $arg.clone() {
                    $item = Some(arg);
                }
            };
        }

        apply!(self.initramfs, &args.initramfs);
        apply!(self.ovmf, &args.ovmf);
        apply!(self.opensbi, &args.opensbi);
        apply!(self.grub_mkrescue, &args.grub_mkrescue);
        apply!(self.bootloader, &args.bootloader);
        apply!(self.boot_protocol, &args.boot_protocol);
        apply!(self.qemu_exe, &args.qemu_exe);

        apply_kv_array(&mut self.kcmd_args, &args.kcmd_args, "=", &[]);
        for init_arg in &args.init_args {
            for seperated_arg in init_arg.split(' ') {
                self.init_args.push(seperated_arg.to_string());
            }
        }

        qemu::apply_qemu_args_addition(&mut self.qemu_args, &args.qemu_args_add);
    }

    pub fn combined_kcmd_args(&self) -> Vec<String> {
        let mut kcmd_args = self.kcmd_args.clone();
        kcmd_args.push("--".to_owned());
        kcmd_args.extend(self.init_args.clone());
        kcmd_args
    }
}
