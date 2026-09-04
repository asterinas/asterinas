// SPDX-License-Identifier: MPL-2.0

pub mod bin;
pub mod file;
pub mod vm_image;

use bin::{AsterBin, AsterBinType};
use file::{BundleFile, Initramfs};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    path::{Path, PathBuf},
    process::{self, ExitStatus},
    sync::atomic::AtomicI32,
    time::{Duration, SystemTime},
};
use tempfile::NamedTempFile;
use vm_image::{AsterVmImage, AsterVmImageType};

const QEMU_MONITOR_STARTUP_ATTEMPTS: usize = 20;
const QEMU_MONITOR_STARTUP_INTERVAL: Duration = Duration::from_millis(50);

use crate::{
    arch::Arch,
    config::{
        Config,
        scheme::{Action, ActionChoice, BootMethod, BootProtocol},
    },
    error::Errno,
    error_msg,
    signal::{SignalGuard, signal_value, wait_for_child},
    util::{DirGuard, new_command_checked_exists},
    virtiofs::{VirtioFsGuard, VirtioFsStartError},
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
#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub struct BundleManifest {
    pub initramfs: Option<Initramfs>,
    pub aster_bin: Option<AsterBin>,
    pub vm_image: Option<AsterVmImage>,
    pub config: Config,
    pub action: ActionChoice,
    pub last_modified: SystemTime,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum QemuExit {
    Success,
    Failed,
    Unknown,
}

pub(crate) fn classify_qemu_exit_status(exit_status: ExitStatus) -> QemuExit {
    if exit_status.success() {
        return QemuExit::Success;
    }

    let Some(qemu_exit_code) = exit_status.code() else {
        return QemuExit::Unknown;
    };

    // For x86 QEMU with `isa-debug-exit`, the guest exit code is encoded as
    // `(code << 1) | 1`. Do not decode QEMU's own failure exit code `1`.
    if qemu_exit_code == 1 {
        return QemuExit::Unknown;
    }

    let kernel_exit_code = qemu_exit_code >> 1;
    match kernel_exit_code {
        // Corresponds to `ostd::QemuExitCode::Success`.
        0x10 => QemuExit::Success,
        // Corresponds to `ostd::QemuExitCode::Failed`.
        0x20 => QemuExit::Failed,
        // Unknown exit code, e.g., a triple fault.
        _ => QemuExit::Unknown,
    }
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

        let _dir_guard = DirGuard::change_dir(&path);

        if let Some(aster_bin) = &manifest.aster_bin
            && !aster_bin.validate()
        {
            return None;
        }
        if let Some(vm_image) = &manifest.vm_image
            && !vm_image.validate()
        {
            return None;
        }
        if let Some(initramfs) = &manifest.initramfs
            && !initramfs.validate()
        {
            return None;
        }

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

                // Validate the kernel binary type against the configured boot protocol.
                // This prevents reusing an incompatible binary (e.g. ELF vs. `bzImage`) when
                // switching boot methods (for example, from a Grub ISO to `qemu-direct`),
                // which would otherwise cause boot failures.
                let aster_bin_type = self.manifest.aster_bin.as_ref().unwrap().typ();
                let expects_linux = matches!(aster_bin_type, AsterBinType::BzImage(_));
                let actual_linux = config_action.grub.boot_protocol == BootProtocol::Linux;
                if expects_linux != actual_linux {
                    return Err(
                        "The boot protocol is not compatible with the kernel binary".to_owned()
                    );
                }
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
                if initramfs.size() != config_initramfs.size()
                    || initramfs.modified_time() < config_initramfs.modified_time()
                {
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
        let exit_status = match self.run_qemu_and_wait(config, action) {
            Ok(exit_status) => exit_status,
            Err(errno) => process::exit(errno as _),
        };

        // FIXME: When panicking it sometimes returns success, why?
        match classify_qemu_exit_status(exit_status) {
            QemuExit::Success => {}
            QemuExit::Failed => std::process::exit(1),
            QemuExit::Unknown => std::process::exit(2),
        }
    }

    /// Returns the QEMU status, or an OSDK error code for the caller to handle.
    pub(crate) fn run_qemu_and_wait(
        &self,
        config: &Config,
        action: ActionChoice,
    ) -> Result<ExitStatus, Errno> {
        match self.can_run_with_config(config, action) {
            Ok(()) => {}
            Err(msg) => {
                error_msg!("{}", msg);
                return Err(Errno::RunBundle);
            }
        }

        let action = match action {
            ActionChoice::Run => &config.run,
            ActionChoice::Test => &config.test,
        };
        let qemu_cmd = self.build_qemu_command(config, action)?;

        let (signal_guard, _virtiofs_guard) = setup_qemu_runtime(config, action)?;
        let signal = signal_guard.signal();

        if action.qemu.with_monitor && action.qemu.log_file.is_some() {
            self.run_qemu_with_monitor(config, action, qemu_cmd, signal)
        } else {
            self.run_qemu_direct(config, action, qemu_cmd, signal)
        }
    }

    fn run_qemu_with_monitor(
        &self,
        config: &Config,
        action: &Action,
        mut qemu_cmd: process::Command,
        signal: &AtomicI32,
    ) -> Result<ExitStatus, Errno> {
        fn wait_until_guest_kernel_shutdown(
            config: &Config,
            qemu_log_path: &Path,
            qemu_monitor_stream: &mut UnixStream,
            signal: &AtomicI32,
        ) -> Result<(), Errno> {
            let mut monitor_reader = BufReader::new(&mut *qemu_monitor_stream);
            let mut monitor_line = Vec::new();

            // Check VM status every 0.1 seconds and break the loop if the VM is stopped or hanging.
            while monitor_reader.get_mut().write_all(b"info status\n").is_ok() {
                if signal_value(signal).is_some() {
                    return Err(Errno::Interrupted);
                }

                match monitor_reader.read_until(b'\n', &mut monitor_line) {
                    Ok(_) => {
                        if String::from_utf8_lossy(&monitor_line).trim_end_matches(['\r', '\n'])
                            == "VM status: paused (shutdown)"
                        {
                            break;
                        }
                        monitor_line.clear();
                    }
                    Err(err) => {
                        if !matches!(
                            err.kind(),
                            std::io::ErrorKind::TimedOut | std::io::ErrorKind::WouldBlock
                        ) {
                            monitor_line.clear();
                        }
                    }
                }

                if config.target_arch == Arch::RiscV64
                    && let Ok(log_file) = std::fs::File::open(qemu_log_path)
                {
                    let log = rev_buf_reader::RevBufReader::new(&log_file);
                    if log.lines().next().is_some_and(|line| {
                        line.as_ref().is_ok_and(|s| {
                            s.contains("SBI system_reset cannot shut down the underlying machine")
                        })
                    }) {
                        break;
                    }
                }
                std::thread::sleep(Duration::from_millis(100));
            }
            Ok(())
        }

        let qemu_log_path = config.work_dir.join(action.qemu.log_file.as_ref().unwrap());
        let qemu_monitor_socket_path = NamedTempFile::new().unwrap().into_temp_path();
        qemu_cmd.arg("-monitor").arg(format!(
            "unix:{},server,nowait",
            qemu_monitor_socket_path.to_string_lossy()
        ));

        info!("Running QEMU: {qemu_cmd:#?}");

        let mut qemu_child = match qemu_cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                error_msg!("failed to start QEMU: {}", err);
                return Err(Errno::ExecuteCommand);
            }
        };

        for _ in 0..QEMU_MONITOR_STARTUP_ATTEMPTS {
            if signal_value(signal).is_some() {
                let _ = qemu_child.kill();
                let _ = qemu_child.wait();
                return Err(Errno::Interrupted);
            }
            std::thread::sleep(QEMU_MONITOR_STARTUP_INTERVAL);
        }

        let mut qemu_monitor_stream = match UnixStream::connect(&qemu_monitor_socket_path) {
            Ok(stream) => stream,
            Err(err) => {
                let _ = qemu_child.kill();
                let _ = qemu_child.wait();

                error_msg!(
                    "failed to connect to QEMU monitor `{}`: {}",
                    qemu_monitor_socket_path.display(),
                    err
                );

                return Err(Errno::ExecuteCommand);
            }
        };

        qemu_monitor_stream
            .set_read_timeout(Some(Duration::from_millis(100)))
            .unwrap();

        let signal_result = wait_until_guest_kernel_shutdown(
            config,
            &qemu_log_path,
            &mut qemu_monitor_stream,
            signal,
        );

        qemu_monitor_stream.set_read_timeout(None).unwrap();
        if let Err(errno) = signal_result {
            let _ = qemu_child.kill();
            let _ = qemu_child.wait();
            return Err(errno);
        }

        info!("VM is paused (shutdown)");

        self.post_run_action(config, action, Some(&mut qemu_monitor_stream));

        let _ = qemu_monitor_stream.write_all(b"quit\n");
        wait_for_child(&mut qemu_child, signal)
    }

    fn run_qemu_direct(
        &self,
        config: &Config,
        action: &Action,
        mut qemu_cmd: process::Command,
        signal: &AtomicI32,
    ) -> Result<ExitStatus, Errno> {
        info!("Running QEMU: {qemu_cmd:#?}");

        let mut qemu_child = match qemu_cmd.spawn() {
            Ok(child) => child,
            Err(err) => {
                error_msg!("failed to start QEMU: {}", err);
                return Err(Errno::ExecuteCommand);
            }
        };

        let exit_status = wait_for_child(&mut qemu_child, signal)?;
        self.post_run_action(config, action, None);
        Ok(exit_status)
    }

    /// Move the vm_image into the bundle.
    pub fn consume_vm_image(&mut self, vm_image: AsterVmImage) {
        if self.manifest.vm_image.is_some() {
            panic!("vm_image already exists");
        }
        self.manifest.vm_image = Some(vm_image.copy_to(&self.path));
        self.write_manifest_to_fs();
    }

    /// Move the aster_bin into the bundle.
    pub fn consume_aster_bin(&mut self, aster_bin: AsterBin) {
        if self.manifest.aster_bin.is_some() {
            panic!("aster_bin already exists");
        }
        self.manifest.aster_bin = Some(aster_bin.copy_to(&self.path));
        self.write_manifest_to_fs();
    }

    fn write_manifest_to_fs(&mut self) {
        self.manifest.last_modified = SystemTime::now();
        let manifest_file_content = toml::to_string(&self.manifest).unwrap();
        let manifest_file_path = self.path.join("bundle.toml");
        std::fs::write(manifest_file_path, manifest_file_content).unwrap();
    }

    fn build_qemu_command(
        &self,
        config: &Config,
        action: &Action,
    ) -> Result<process::Command, Errno> {
        let mut qemu_cmd = new_command_checked_exists(&action.qemu.path);
        qemu_cmd.current_dir(&config.work_dir);

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
                let bootdev_opts = action
                    .qemu
                    .bootdev_append_options
                    .as_deref()
                    .unwrap_or(",index=2,media=cdrom");
                qemu_cmd.arg("-drive").arg(format!(
                    "file={},format=raw{}",
                    self.path.join(vm_image.path()).to_string_lossy(),
                    bootdev_opts,
                ));
            }
            BootMethod::GrubQcow2 => {
                let vm_image = self.manifest.vm_image.as_ref().unwrap();
                assert!(matches!(vm_image.typ(), AsterVmImageType::Qcow2(_)));
                // FIXME: this doesn't work for regular QEMU, but may work for TDX.
                let bootdev_opts = action
                    .qemu
                    .bootdev_append_options
                    .as_deref()
                    .unwrap_or(",if=virtio");
                qemu_cmd.arg("-drive").arg(format!(
                    "file={},format=qcow2{}",
                    self.path.join(vm_image.path()).to_string_lossy(),
                    bootdev_opts,
                ));
            }
        };

        match shlex::split(&action.qemu.args) {
            Some(v) => {
                for arg in v {
                    qemu_cmd.arg(arg);
                }
            }
            None => {
                error_msg!("Failed to parse qemu args: {:#?}", &action.qemu.args);
                return Err(Errno::ParseMetadata);
            }
        };

        Ok(qemu_cmd)
    }

    fn post_run_action(
        &self,
        config: &Config,
        action: &Action,
        qemu_monitor_stream: Option<&mut UnixStream>,
    ) {
        let Some(qemu_log_file) = &action.qemu.log_file else {
            return;
        };

        // Read the configured QEMU output and check if it failed with a panic.
        // Setting a QEMU log is required for source line stack trace because piping the output
        // is less desirable when running QEMU with serial redirected to standard I/O.
        let qemu_log_path = config.work_dir.join(qemu_log_file);
        if let Ok(file) = std::fs::File::open(&qemu_log_path)
            && let Some(aster_bin) = &self.manifest.aster_bin
        {
            crate::util::trace_panic_from_log(file, self.path.join(aster_bin.path()));
        }

        // Find the coverage data information in the QEMU log, and dump it if found.
        if let Some(qemu_monitor_stream) = qemu_monitor_stream
            && let Ok(file) = std::fs::File::open(&qemu_log_path)
        {
            crate::util::dump_coverage_from_qemu(file, qemu_monitor_stream);
        }
    }
}

fn setup_qemu_runtime(
    config: &Config,
    action: &Action,
) -> Result<(SignalGuard, Option<VirtioFsGuard>), Errno> {
    let signal_guard = match SignalGuard::install() {
        Ok(guard) => guard,
        Err(err) => {
            error_msg!("failed to install signal handlers: {err}");
            return Err(Errno::ExecuteCommand);
        }
    };
    let signal = signal_guard.signal();

    let virtiofs_guard = match VirtioFsGuard::start(
        &config.work_dir,
        action.qemu.virtiofs.as_ref(),
        &action.virtiofsd,
        signal,
    ) {
        Ok(guard) => guard,
        Err(VirtioFsStartError::Interrupted) => {
            return Err(Errno::Interrupted);
        }
        Err(VirtioFsStartError::Failed) => {
            error_msg!("failed to start virtiofsd");
            return Err(Errno::ExecuteCommand);
        }
    };

    Ok((signal_guard, virtiofs_guard))
}
