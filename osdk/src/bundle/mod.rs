// SPDX-License-Identifier: MPL-2.0

pub mod bin;
pub mod file;
pub mod vm_image;

use bin::AsterBin;
use file::{BundleFile, Initramfs};
use std::process;
use vm_image::{AsterVmImage, AsterVmImageType};

use std::{
    path::{Path, PathBuf},
    process::Command,
    time::SystemTime,
};

use crate::{
    config::{
        scheme::{ActionChoice, BootMethod},
        Config,
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
    pub initramfs: Option<Initramfs>,
    pub aster_bin: Option<AsterBin>,
    pub vm_image: Option<AsterVmImage>,
    pub config: Config,
    pub action: ActionChoice,
    pub last_modified: SystemTime,
}

impl Bundle {
    /// This function creates a new `Bundle` without adding any files.
    pub fn new(path: impl AsRef<Path>, config: &Config, action: ActionChoice) -> Self {
        std::fs::create_dir_all(path.as_ref()).unwrap();
        let config_initramfs = match action {
            ActionChoice::Run => config.run.boot.initramfs.as_ref(),
            ActionChoice::Test => config.test.boot.initramfs.as_ref(),
        };
        let initramfs = if let Some(ref initramfs) = config_initramfs {
            if !initramfs.exists() {
                error_msg!("initramfs file not found: {}", initramfs.display());
                process::exit(Errno::BuildCrate as _);
            }
            Some(Initramfs::new(initramfs).copy_to(&path))
        } else {
            None
        };
        let mut created = Self {
            manifest: BundleManifest {
                initramfs,
                aster_bin: None,
                vm_image: None,
                config: config.clone(),
                action,
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

    pub fn can_run_with_config(&self, config: &Config, action: ActionChoice) -> Result<(), String> {
        // If built for testing, better not to run it. Vice versa.
        if self.manifest.action != action {
            return Err(format!(
                "The bundle is built for {:?}",
                self.manifest.action
            ));
        }

        let self_action = match self.manifest.action {
            ActionChoice::Run => &self.manifest.config.run,
            ActionChoice::Test => &self.manifest.config.test,
        };
        let config_action = match action {
            ActionChoice::Run => &config.run,
            ActionChoice::Test => &config.test,
        };

        // Compare the manifest with the run configuration except the initramfs and the boot method.
        if self_action.grub != config_action.grub
            || self_action.qemu != config_action.qemu
            || self_action.build != config_action.build
            || self_action.boot.kcmdline != config_action.boot.kcmdline
        {
            return Err("The bundle is not compatible with the run configuration".to_owned());
        }

        // Checkout if the files on disk supports the boot method
        match config_action.boot.method {
            BootMethod::QemuDirect => {
                if self.manifest.aster_bin.is_none() {
                    return Err("Kernel binary is required for direct QEMU booting".to_owned());
                };
            }
            BootMethod::GrubRescueIso => {
                let Some(ref vm_image) = self.manifest.vm_image else {
                    return Err("VM image is required for QEMU booting".to_owned());
                };
                if !matches!(vm_image.typ(), AsterVmImageType::GrubIso(_)) {
                    return Err("VM image in the bundle is not a Grub ISO image".to_owned());
                }
            }
            BootMethod::GrubQcow2 => {
                let Some(ref vm_image) = self.manifest.vm_image else {
                    return Err("VM image is required for QEMU booting".to_owned());
                };
                if !matches!(vm_image.typ(), AsterVmImageType::Qcow2(_)) {
                    return Err("VM image in the bundle is not a Qcow2 image".to_owned());
                }
            }
        }

        // Compare the initramfs.
        let initramfs_err =
            "The initramfs in the bundle is different from the one in the run configuration"
                .to_owned();
        match (&self.manifest.initramfs, &config_action.boot.initramfs) {
            (Some(initramfs), Some(initramfs_path)) => {
                let config_initramfs = Initramfs::new(initramfs_path);
                if initramfs.sha256sum() != config_initramfs.sha256sum() {
                    return Err(initramfs_err);
                }
            }
            (None, None) => {}
            _ => {
                return Err(initramfs_err);
            }
        };

        Ok(())
    }

    pub fn last_modified_time(&self) -> SystemTime {
        self.manifest.last_modified
    }

    pub fn run(&self, config: &Config, action: ActionChoice) {
        match self.can_run_with_config(config, action) {
            Ok(()) => {}
            Err(msg) => {
                error_msg!("{}", msg);
                std::process::exit(Errno::RunBundle as _);
            }
        }
        let action = match action {
            ActionChoice::Run => &config.run,
            ActionChoice::Test => &config.test,
        };
        let mut qemu_cmd = Command::new(&action.qemu.path);
        match shlex::split(&action.qemu.args) {
            Some(v) => {
                for arg in v {
                    qemu_cmd.arg(arg);
                }
            }
            None => {
                error_msg!("Failed to parse qemu args: {:#?}", &action.qemu.args);
                process::exit(Errno::ParseMetadata as _);
            }
        }
        match action.boot.method {
            BootMethod::QemuDirect => {
                let aster_bin = self.manifest.aster_bin.as_ref().unwrap();
                qemu_cmd
                    .arg("-kernel")
                    .arg(self.path.join(aster_bin.path()));
                if let Some(ref initramfs) = action.boot.initramfs {
                    qemu_cmd.arg("-initrd").arg(initramfs);
                } else {
                    info!("No initramfs specified");
                };
                qemu_cmd.arg("-append").arg(action.boot.kcmdline.join(" "));
            }
            BootMethod::GrubRescueIso => {
                let vm_image = self.manifest.vm_image.as_ref().unwrap();
                assert!(matches!(vm_image.typ(), AsterVmImageType::GrubIso(_)));
                qemu_cmd.arg("-cdrom").arg(self.path.join(vm_image.path()));
            }
            BootMethod::GrubQcow2 => {
                let vm_image = self.manifest.vm_image.as_ref().unwrap();
                assert!(matches!(vm_image.typ(), AsterVmImageType::Qcow2(_)));
                qemu_cmd.arg("-drive").arg(format!(
                    "file={},index=0,media=disk,format=qcow2",
                    self.path
                        .join(vm_image.path())
                        .into_os_string()
                        .into_string()
                        .unwrap()
                ));
            }
        };

        info!("Running QEMU: {:#?}", qemu_cmd);

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

    fn write_manifest_to_fs(&mut self) {
        self.manifest.last_modified = SystemTime::now();
        let manifest_file_content = toml::to_string(&self.manifest).unwrap();
        let manifest_file_path = self.path.join("bundle.toml");
        std::fs::write(manifest_file_path, manifest_file_content).unwrap();
    }
}
