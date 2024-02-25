// SPDX-License-Identifier: MPL-2.0

use std::{path::PathBuf, process};

use crate::{error::Errno, error_msg};

/// Arguments for creating bootdev image and how to boot with vmm.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub struct Boot {
    #[serde(default)]
    pub loader: BootLoader,
    #[serde(default)]
    pub protocol: BootProtocol,
    /// The path of `grub_mkrecue`. Only be `Some(_)` if `loader` is `BootLoader::grub`.
    pub grub_mkrescue: Option<PathBuf>,
    /// The path of ovmf. Only be `Some(_)` if `protocol` is `BootProtocol::LinuxEfiHandover64`.
    pub ovmf: Option<PathBuf>,
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BootLoader {
    #[default]
    Grub,
    Qemu,
}

impl<'a> From<&'a str> for BootLoader {
    fn from(value: &'a str) -> Self {
        match value {
            "grub" => Self::Grub,
            "qemu" => Self::Qemu,
            _ => {
                error_msg!("`{}` is not a valid option for `boot.loader`. Allowed options are `grub` and `qemu`.",value);
                process::exit(Errno::ParseMetadata as _);
            }
        }
    }
}

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum BootProtocol {
    LinuxEfiHandover64,
    LinuxLegacy32,
    Multiboot,
    #[default]
    Multiboot2,
}

impl<'a> From<&'a str> for BootProtocol {
    fn from(value: &'a str) -> Self {
        match value {
            "linux-efi-handover64" => Self::LinuxEfiHandover64,
            "linux-legacy32" => Self::LinuxLegacy32,
            "multiboot" => Self::Multiboot,
            "multiboot2" => Self::Multiboot2,
            _ => {
                error_msg!("`{}` is not a valid option for `boot.protocol`. Allowed options are `linux-efi-handover64`, `linux-legacy32`, `multiboot`, `multiboot2`", value);
                process::exit(Errno::ParseMetadata as _);
            }
        }
    }
}
