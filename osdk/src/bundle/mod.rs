// SPDX-License-Identifier: MPL-2.0

pub mod bin;
pub mod file;
pub mod vm_image;

use bin::{AsterBin, AsterBinType};
use file::{BundleFile, Initramfs};
use std::{
    io::{BufRead, BufReader, Write},
    os::unix::net::UnixStream,
    process::{self, Child, Command, ExitStatus},
    time::Duration,
};
use tempfile::NamedTempFile;
use vm_image::{AsterVmImage, AsterVmImageType};

use std::{
    path::{Path, PathBuf},
    time::SystemTime,
};

use crate::{
    arch::Arch,
    config::{
        Config,
        scheme::{Action, ActionChoice, BootMethod, BootProtocol},
    },
    error::Errno,
    error_msg,
    program_supervisor::{self, ProgramSupervisor},
    signal::{self, SignalGuard},
    util::{DirGuard, new_command_checked_exists},
    warn_msg,
};

const MONITOR_POLL_INTERVAL: Duration = Duration::from_millis(100);

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
        let exit_status = self.run_qemu_and_wait(config, action);
        // FIXME: When panicking it sometimes returns success, why?
        match classify_qemu_exit_status(exit_status) {
            QemuExit::Success => {}
            QemuExit::Failed => std::process::exit(1),
            QemuExit::Unknown => std::process::exit(2),
        }
    }

    pub(crate) fn run_qemu_and_wait(&self, config: &Config, action: ActionChoice) -> ExitStatus {
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

        let signal_guard = SignalGuard::install().unwrap_or_else(|err| {
            error_msg!("failed to install signal handlers: {err}");
            process::exit(Errno::ExecuteCommand as _);
        });
        let signal = signal_guard.signal();

        let qemu_cmd = self.build_qemu_command(config, action);
        let mut process_supervisor =
            ProgramSupervisor::start(&action.qemu.programs, &config.work_dir, signal)
                .unwrap_or_else(|errno| process::exit(errno as _));

        let qemu_result = self.run_qemu(config, action, qemu_cmd, signal, &mut process_supervisor);

        process_supervisor.stop_all();
        qemu_result.unwrap_or_else(|errno| process::exit(errno as _))
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

    fn process_qemu_log(
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

    fn build_qemu_command(&self, config: &Config, action: &Action) -> Command {
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
            Some(args) => {
                for arg in args {
                    qemu_cmd.arg(arg);
                }
            }
            None => {
                error_msg!("Failed to parse qemu args: {:#?}", &action.qemu.args);
                process::exit(Errno::ParseMetadata as _);
            }
        }

        qemu_cmd
    }

    fn run_qemu(
        &self,
        config: &Config,
        action: &Action,
        qemu_cmd: Command,
        signal: &std::sync::atomic::AtomicI32,
        process_supervisor: &mut ProgramSupervisor<'_>,
    ) -> Result<ExitStatus, Errno> {
        if action.qemu.with_monitor && action.qemu.log_file.is_some() {
            self.run_qemu_with_monitor(config, action, qemu_cmd, signal, process_supervisor)
        } else {
            self.run_qemu_without_monitor(config, action, qemu_cmd, process_supervisor)
        }
    }

    fn run_qemu_with_monitor(
        &self,
        config: &Config,
        action: &Action,
        mut qemu_cmd: Command,
        signal: &std::sync::atomic::AtomicI32,
        process_supervisor: &mut ProgramSupervisor<'_>,
    ) -> Result<ExitStatus, Errno> {
        let qemu_log_file = action.qemu.log_file.as_ref().unwrap();
        let qemu_log_path = config.work_dir.join(qemu_log_file);
        let qemu_monitor_socket_path = NamedTempFile::new().unwrap().into_temp_path();
        qemu_cmd.arg("-monitor").arg(format!(
            "unix:{},server,nowait",
            qemu_monitor_socket_path.to_string_lossy()
        ));

        let mut qemu_child = Self::spawn_qemu(&mut qemu_cmd)?;
        std::thread::sleep(Duration::from_secs(1)); // Wait for QEMU to start
        let mut qemu_monitor_stream =
            UnixStream::connect(&qemu_monitor_socket_path).map_err(|err| {
                error_msg!("failed to connect to QEMU monitor: {err}");
                program_supervisor::stop_qemu(&mut qemu_child);
                Errno::ExecuteCommand
            })?;
        if let Err(errno) = wait_until_guest_kernel_shutdown(
            config,
            &qemu_log_path,
            &mut qemu_monitor_stream,
            &mut qemu_child,
            signal,
            process_supervisor,
        ) {
            program_supervisor::stop_qemu(&mut qemu_child);
            return Err(errno);
        }
        info!("VM is paused (shutdown)");

        self.process_qemu_log(config, action, Some(&mut qemu_monitor_stream));

        let _ = qemu_monitor_stream.write_all(b"quit\n");
        process_supervisor.wait_for_qemu(&mut qemu_child)
    }

    fn run_qemu_without_monitor(
        &self,
        config: &Config,
        action: &Action,
        mut qemu_cmd: Command,
        process_supervisor: &mut ProgramSupervisor<'_>,
    ) -> Result<ExitStatus, Errno> {
        let mut qemu_child = Self::spawn_qemu(&mut qemu_cmd)?;
        let exit_status = match process_supervisor.wait_for_qemu(&mut qemu_child) {
            Ok(status) => status,
            Err(errno) => {
                program_supervisor::stop_qemu(&mut qemu_child);
                return Err(errno);
            }
        };

        self.process_qemu_log(config, action, None);

        Ok(exit_status)
    }

    fn spawn_qemu(qemu_cmd: &mut Command) -> Result<Child, Errno> {
        info!("Running QEMU: {qemu_cmd:#?}");
        qemu_cmd.spawn().map_err(|err| {
            error_msg!("failed to start QEMU: {err}");
            Errno::ExecuteCommand
        })
    }
}

fn wait_until_guest_kernel_shutdown(
    config: &Config,
    qemu_log_path: &Path,
    qemu_monitor_stream: &mut UnixStream,
    qemu_child: &mut Child,
    signal: &std::sync::atomic::AtomicI32,
    process_supervisor: &mut ProgramSupervisor<'_>,
) -> Result<(), Errno> {
    qemu_monitor_stream
        .set_read_timeout(Some(MONITOR_POLL_INTERVAL))
        .map_err(|err| {
            warn_msg!("failed to configure QEMU monitor timeout: {err}");
            Errno::ExecuteCommand
        })?;

    let mut monitor_reader = BufReader::new(&mut *qemu_monitor_stream);
    let mut line = String::new();

    // Check VM status every 0.1 seconds and break the loop if the VM is stopped or hanging.
    loop {
        if signal::signal_value(signal).is_some() {
            program_supervisor::stop_qemu(qemu_child);
            return Err(Errno::Interrupted);
        }

        process_supervisor.check_children()?;

        if qemu_child
            .try_wait()
            .map_err(|err| {
                warn_msg!("failed to poll QEMU: {err}");
                Errno::ExecuteCommand
            })?
            .is_some()
        {
            break;
        }

        if monitor_reader
            .get_mut()
            .write_all(b"info status\n")
            .is_err()
        {
            break;
        }

        let mut guest_shutdown = false;

        loop {
            line.clear();
            match monitor_reader.read_line(&mut line) {
                Ok(bytes_read) => {
                    if bytes_read == 0 {
                        break;
                    }
                    if line.starts_with("VM status:") {
                        guest_shutdown = line.trim_end() == "VM status: paused (shutdown)";
                        break;
                    }
                }
                Err(_) => break,
            }
        }

        if guest_shutdown {
            break;
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

        std::thread::sleep(MONITOR_POLL_INTERVAL);
    }

    Ok(())
}
