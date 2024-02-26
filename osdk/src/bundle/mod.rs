// SPDX-License-Identifier: MPL-2.0

pub mod bin;
pub mod file;
pub mod vm_image;

use bin::AsterBin;
use file::{BundleFile, Initramfs};
use vm_image::AsterVmImage;

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use crate::{
    cli::CargoArgs,
    config_manager::{
        boot::Boot,
        qemu::{Qemu, QemuMachine},
        RunConfig,
    },
    error::Errno,
    error_msg,
};

/// The osdk bundle artifact that stores as `bundle` directory.
///
/// This `Bundle` struct is used to track a bundle on a filesystem. Every modification to the bundle
/// would result in file system writes. But the bundle will not be removed from the file system when
/// the `Bundle` is dropped.
pub struct Bundle {
    manifest: BundleManifest,
    path: PathBuf,
}

/// The osdk bundle artifact manifest that stores as `bundle.toml`.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
    pub kcmd_args: Vec<String>,
    pub initramfs: Option<Initramfs>,
    pub aster_bin: Option<AsterBin>,
    pub vm_image: Option<AsterVmImage>,
    pub boot: Boot,
    pub qemu: Qemu,
    pub cargo_args: CargoArgs,
    pub last_modified: SystemTime,
}

impl Bundle {
    /// This function creates a new `Bundle` without adding any files.
    pub fn new(
        path: impl AsRef<Path>,
        kcmd_args: Vec<String>,
        boot: Boot,
        qemu: Qemu,
        cargo_args: CargoArgs,
    ) -> Self {
        std::fs::create_dir_all(path.as_ref()).unwrap();
        let mut created = Self {
            manifest: BundleManifest {
                kcmd_args,
                initramfs: None,
                aster_bin: None,
                vm_image: None,
                boot,
                qemu,
                cargo_args,
                last_modified: SystemTime::now(),
            },
            path: path.as_ref().to_path_buf(),
        };
        created.write_manifest_to_fs();
        created
    }

    // Load the bundle from the file system. If the bundle does not exist or have inconsistencies,
    // it will return `None`.
    pub fn load(path: impl AsRef<Path>) -> Option<Self> {
        let manifest_file_path = path.as_ref().join("bundle.toml");
        let manifest_file_content = std::fs::read_to_string(manifest_file_path).ok()?;
        let manifest: BundleManifest = toml::from_str(&manifest_file_content).ok()?;

        let original_dir = std::env::current_dir().unwrap();
        std::env::set_current_dir(&path).unwrap();

        if let Some(aster_bin) = &manifest.aster_bin {
            if !aster_bin.validate() {
                return None;
            }
        }
        if let Some(vm_image) = &manifest.vm_image {
            if !vm_image.validate() {
                return None;
            }
        }
        if let Some(initramfs) = &manifest.initramfs {
            if !initramfs.validate() {
                return None;
            }
        }

        std::env::set_current_dir(original_dir).unwrap();

        Some(Self {
            manifest,
            path: path.as_ref().to_path_buf(),
        })
    }

    pub fn can_run_with_config(&self, config: &RunConfig) -> bool {
        // Compare the manifest with the run configuration.
        // TODO: This pairwise comparison will result in some false negatives. We may
        // fix it by pondering upon each fields with more care.
        if self.manifest.kcmd_args != config.manifest.kcmd_args
            || self.manifest.boot != config.manifest.boot
            || self.manifest.qemu != config.manifest.qemu
            || self.manifest.cargo_args != config.cargo_args
        {
            return false;
        }

        // Compare the initramfs.
        match (&self.manifest.initramfs, &config.manifest.initramfs) {
            (Some(initramfs), Some(initramfs_path)) => {
                let config_initramfs = Initramfs::new(initramfs_path);
                if initramfs.sha256sum() != config_initramfs.sha256sum() {
                    return false;
                }
            }
            (None, None) => {}
            _ => {
                return false;
            }
        };

        true
    }

    pub fn last_modified_time(&self) -> SystemTime {
        self.manifest.last_modified
    }

    pub fn run(&self, config: &RunConfig) {
        if !self.can_run_with_config(config) {
            error_msg!("The bundle is not compatible with the run configuration");
            std::process::exit(Errno::RunBundle as _);
        }
        let mut qemu_cmd = Command::new(config.manifest.qemu.path.clone().unwrap());
        // FIXME: Arguments like "-m 2G" sould be separated into "-m" and "2G". This
        // is a dirty hack to make it work. Anything like space in the paths will
        // break this.
        for arg in &config.manifest.qemu.args {
            for part in arg.split_whitespace() {
                qemu_cmd.arg(part);
            }
        }
        match config.manifest.qemu.machine {
            QemuMachine::Microvm => {
                qemu_cmd.arg("-machine").arg("microvm");
                let Some(ref aster_bin) = self.manifest.aster_bin else {
                    error_msg!("Kernel ELF binary is required for Microvm");
                    std::process::exit(Errno::RunBundle as _);
                };
                qemu_cmd
                    .arg("-kernel")
                    .arg(self.path.join(aster_bin.path()));
                let Some(ref initramfs) = config.manifest.initramfs else {
                    error_msg!("Initramfs is required for Microvm");
                    std::process::exit(Errno::RunBundle as _);
                };
                qemu_cmd.arg("-initrd").arg(initramfs);
                qemu_cmd
                    .arg("-append")
                    .arg(config.manifest.kcmd_args.join(" "));
            }
            QemuMachine::Q35 => {
                qemu_cmd.arg("-machine").arg("q35,kernel-irqchip=split");
                let Some(ref vm_image) = self.manifest.vm_image else {
                    error_msg!("VM image is required for QEMU booting");
                    std::process::exit(Errno::RunBundle as _);
                };
                qemu_cmd.arg("-cdrom").arg(self.path.join(vm_image.path()));
                if let Some(ovmf) = &config.manifest.boot.ovmf {
                    qemu_cmd.arg("-drive").arg(format!(
                        "if=pflash,format=raw,unit=0,readonly=on,file={}",
                        ovmf.join("OVMF_CODE.fd").display()
                    ));
                    qemu_cmd.arg("-drive").arg(format!(
                        "if=pflash,format=raw,unit=1,file={}",
                        ovmf.join("OVMF_VARS.fd").display()
                    ));
                }
            }
        };
        qemu_cmd.arg("-cpu").arg("Icelake-Server,+x2apic");

        for drive_file in &config.manifest.qemu.drive_files {
            qemu_cmd.arg("-drive").arg(format!(
                "file={},{}",
                drive_file.path.display(),
                drive_file.append,
            ));
        }

        let exit_status = qemu_cmd.status().unwrap();
        if !exit_status.success() {
            // FIXME: Exit code manipulation is not needed when using non-x86 QEMU
            let qemu_exit_code = exit_status.code().unwrap();
            let kernel_exit_code = qemu_exit_code >> 1;
            match kernel_exit_code {
                0x10 /*aster_frame::QemuExitCode::Success*/ => { std::process::exit(0); },
                0x20 /*aster_frame::QemuExitCode::Failed*/ => { std::process::exit(1); },
                _ /* unknown, e.g., a triple fault */ => { std::process::exit(2) },
            }
        }
    }

    /// Move the vm_image into the bundle.
    pub fn consume_vm_image(&mut self, vm_image: AsterVmImage) {
        if self.manifest.vm_image.is_some() {
            panic!("vm_image already exists");
        }
        self.manifest.vm_image = Some(vm_image.move_to(&self.path));
        self.write_manifest_to_fs();
    }

    /// Move the aster_bin into the bundle.
    pub fn consume_aster_bin(&mut self, aster_bin: AsterBin) {
        if self.manifest.aster_bin.is_some() {
            panic!("aster_bin already exists");
        }
        self.manifest.aster_bin = Some(aster_bin.move_to(&self.path));
        self.write_manifest_to_fs();
    }

    /// Copy the initramfs into the bundle.
    pub fn add_initramfs(&mut self, initramfs: Initramfs) {
        if self.manifest.initramfs.is_some() {
            panic!("initramfs already exists");
        }
        self.manifest.initramfs = Some(initramfs.copy_to(&self.path));
        self.write_manifest_to_fs();
    }

    fn write_manifest_to_fs(&mut self) {
        self.manifest.last_modified = SystemTime::now();
        let manifest_file_content = toml::to_string(&self.manifest).unwrap();
        let manifest_file_path = self.path.join("bundle.toml");
        std::fs::write(manifest_file_path, manifest_file_content).unwrap();
    }
}
