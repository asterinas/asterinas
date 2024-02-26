// SPDX-License-Identifier: MPL-2.0

use std::{
    path::{Path, PathBuf},
    process::Command,
};

use crate::{
    bin::AsterBin,
    cli::CargoArgs,
    config_manager::{
        boot::Boot,
        qemu::{Qemu, QemuMachine},
        RunConfig,
    },
    error::Errno,
    error_msg,
    vm_image::AsterVmImage,
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

impl Bundle {
    pub fn new(manifest: BundleManifest, path: impl AsRef<Path>) -> Self {
        std::fs::create_dir_all(path.as_ref()).unwrap();
        let created = Self {
            manifest,
            path: path.as_ref().to_path_buf(),
        };
        created.write_manifest_content();
        created
    }

    // FIXME: the load function should be used when implementing build cache, but it is not
    // implemented yet.
    #[allow(dead_code)]
    pub fn load(path: impl AsRef<Path>) -> Self {
        let manifest_file_path = path.as_ref().join("bundle.toml");
        let manifest_file_content = std::fs::read_to_string(manifest_file_path).unwrap();
        let manifest: BundleManifest = toml::from_str(&manifest_file_content).unwrap();
        // TODO: check integrity of the loaded bundle.
        Self {
            manifest,
            path: path.as_ref().to_path_buf(),
        }
    }

    pub fn can_run_with_config(&self, config: &RunConfig) -> bool {
        // TODO: This pairwise comparison will result in some false negatives. We may
        // fix it by pondering upon each fields with more care.
        self.manifest.kcmd_args == config.manifest.kcmd_args
            && self.manifest.initramfs == config.manifest.initramfs
            && self.manifest.boot == config.manifest.boot
            && self.manifest.qemu == config.manifest.qemu
            && self.manifest.cargo_args == config.cargo_args
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
                qemu_cmd.arg("-kernel").arg(self.path.join(&aster_bin.path));
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
                qemu_cmd.arg("-cdrom").arg(self.path.join(&vm_image.path));
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

    pub fn add_vm_image(&mut self, vm_image: &AsterVmImage) {
        if self.manifest.vm_image.is_some() {
            panic!("vm_image already exists");
        }
        let file_name = vm_image.path.file_name().unwrap();
        let copied_path = self.path.join(file_name);
        std::fs::copy(&vm_image.path, copied_path).unwrap();
        self.manifest.vm_image = Some(AsterVmImage {
            path: file_name.into(),
            typ: vm_image.typ.clone(),
            aster_version: vm_image.aster_version.clone(),
            sha256sum: vm_image.sha256sum.clone(),
        });
        self.write_manifest_content();
    }

    pub fn add_aster_bin(&mut self, aster_bin: &AsterBin) {
        if self.manifest.aster_bin.is_some() {
            panic!("aster_bin already exists");
        }
        let file_name = aster_bin.path.file_name().unwrap();
        let copied_path = self.path.join(file_name);
        std::fs::copy(&aster_bin.path, copied_path).unwrap();
        self.manifest.aster_bin = Some(AsterBin {
            path: file_name.into(),
            typ: aster_bin.typ.clone(),
            version: aster_bin.version.clone(),
            sha256sum: aster_bin.sha256sum.clone(),
            stripped: aster_bin.stripped,
        });
        self.write_manifest_content();
    }

    fn write_manifest_content(&self) {
        let manifest_file_content = toml::to_string(&self.manifest).unwrap();
        let manifest_file_path = self.path.join("bundle.toml");
        std::fs::write(manifest_file_path, manifest_file_content).unwrap();
    }
}

/// The osdk bundle artifact manifest that stores as `bundle.toml`.
#[derive(Debug, Default, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct BundleManifest {
    #[serde(default)]
    pub kcmd_args: Vec<String>,
    #[serde(default)]
    pub initramfs: Option<PathBuf>,
    #[serde(default)]
    pub aster_bin: Option<AsterBin>,
    #[serde(default)]
    pub vm_image: Option<AsterVmImage>,
    #[serde(default)]
    pub boot: Boot,
    #[serde(default)]
    pub qemu: Qemu,
    #[serde(default)]
    pub cargo_args: CargoArgs,
}
