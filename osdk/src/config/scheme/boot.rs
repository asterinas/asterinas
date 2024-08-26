// SPDX-License-Identifier: MPL-2.0

use clap::ValueEnum;

use std::path::PathBuf;

#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BootScheme {
    /// Command line arguments for the guest kernel
    #[serde(default)]
    pub kcmd_args: Vec<String>,
    /// Command line arguments for the guest init process
    #[serde(default)]
    pub init_args: Vec<String>,
    /// The path of initramfs
    pub initramfs: Option<PathBuf>,
    /// The infrastructures used to boot the guest
    pub method: Option<BootMethod>,
}

#[derive(Debug, Copy, Clone, Default, PartialEq, Eq, Serialize, Deserialize, ValueEnum)]
#[serde(rename_all = "kebab-case")]
pub enum BootMethod {
    /// Boot the kernel by making a rescue CD image.
    GrubRescueIso,
    /// Boot the kernel by making a Qcow2 image with Grub as the bootloader.
    GrubQcow2,
    /// Use the [QEMU direct boot](https://qemu-project.gitlab.io/qemu/system/linuxboot.html)
    /// to boot the kernel with QEMU's built-in Seabios and Coreboot utilities.
    #[default]
    QemuDirect,
}

#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct Boot {
    pub kcmdline: Vec<String>,
    pub initramfs: Option<PathBuf>,
    pub method: BootMethod,
}

impl BootScheme {
    pub fn inherit(&mut self, from: &Self) {
        self.kcmd_args = {
            let mut kcmd_args = from.kcmd_args.clone();
            kcmd_args.extend(self.kcmd_args.clone());
            kcmd_args
        };
        self.init_args = {
            let mut init_args = from.init_args.clone();
            init_args.extend(self.init_args.clone());
            init_args
        };
        if self.initramfs.is_none() {
            self.initramfs.clone_from(&from.initramfs);
        }
        if self.method.is_none() {
            self.method = from.method;
        }
    }

    pub fn finalize(self) -> Boot {
        let mut kcmdline = self.kcmd_args;
        kcmdline.push("--".to_owned());
        kcmdline.extend(self.init_args);
        Boot {
            kcmdline,
            initramfs: self.initramfs,
            method: self.method.unwrap_or(BootMethod::QemuDirect),
        }
    }
}
