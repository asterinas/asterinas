// SPDX-License-Identifier: MPL-2.0

//! Manages virtio-fs daemon processes used by the QEMU runner.
//!
//! [`VirtioFsGuard`] owns the primary and optional scratch `virtiofsd` children,
//! waits for their vhost-user sockets, and stops them when the QEMU run ends.

use std::{
    fs::{self, File},
    os::unix::fs::FileTypeExt,
    path::{Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::AtomicI32,
    time::Duration,
};

use crate::{
    config::scheme::{VirtioFsScheme, VirtioFsd},
    signal::signal_value,
};

const VIRTIOFSD_STARTUP_ATTEMPTS: usize = 100;

pub(crate) struct VirtioFsGuard {
    primary: VirtioFsProcess,
    scratch: Option<VirtioFsProcess>,
}

struct VirtioFsProcess {
    child: Child,
    socket: PathBuf,
}

pub(crate) enum VirtioFsStartError {
    Failed,
    Interrupted,
}

impl VirtioFsGuard {
    pub(crate) fn start(
        work_dir: &Path,
        virtiofs: Option<&VirtioFsScheme>,
        virtiofsd: &VirtioFsd,
        signal: &AtomicI32,
    ) -> Result<Option<Self>, VirtioFsStartError> {
        let Some(virtiofs) = virtiofs else {
            return Ok(None);
        };
        let primary = start_virtiofsd(
            work_dir,
            virtiofsd,
            virtiofs.socket.clone(),
            virtiofsd.shared_dir.clone(),
            virtiofsd.log_file.clone(),
            signal,
        )?;

        let scratch = if let Some(scratch_socket) = &virtiofs.scratch_socket {
            let scratch = start_virtiofsd(
                work_dir,
                virtiofsd,
                scratch_socket.clone(),
                virtiofsd.scratch_shared_dir.clone(),
                virtiofsd.scratch_log_file.clone(),
                signal,
            );
            match scratch {
                Ok(process) => Some(process),
                Err(err) => {
                    let mut primary = primary;
                    stop_virtiofsd(&mut primary);
                    return Err(err);
                }
            }
        } else {
            None
        };

        Ok(Some(Self { primary, scratch }))
    }
}

impl Drop for VirtioFsGuard {
    fn drop(&mut self) {
        stop_virtiofsd(&mut self.primary);

        if let Some(scratch) = &mut self.scratch {
            stop_virtiofsd(scratch);
        }
    }
}

fn start_virtiofsd(
    work_dir: &Path,
    virtiofsd: &VirtioFsd,
    socket: PathBuf,
    shared_dir: PathBuf,
    log: PathBuf,
    signal: &AtomicI32,
) -> Result<VirtioFsProcess, VirtioFsStartError> {
    let socket = resolve_path(work_dir, socket);
    let shared_dir = resolve_path(work_dir, shared_dir);
    let log = resolve_path(work_dir, log);
    if let Some(parent) = socket.parent() {
        ensure_directory_tree(parent)?;
    }
    ensure_directory_tree(&shared_dir)?;

    if fs::symlink_metadata(&socket).is_ok() {
        return Err(VirtioFsStartError::Failed);
    }

    let log_file = File::create(&log).map_err(|_| VirtioFsStartError::Failed)?;
    let log_stderr = log_file
        .try_clone()
        .map_err(|_| VirtioFsStartError::Failed)?;

    let mut command = Command::new(&virtiofsd.path);

    command
        .current_dir(work_dir)
        .args(&virtiofsd.args)
        .args([
            "--shared-dir",
            shared_dir.to_str().ok_or(VirtioFsStartError::Failed)?,
            "--socket-path",
            socket.to_str().ok_or(VirtioFsStartError::Failed)?,
            "--cache",
            &virtiofsd.cache,
        ])
        .stdout(Stdio::from(log_file))
        .stderr(Stdio::from(log_stderr));
    if virtiofsd.xattr {
        command.arg("--xattr");
    }
    if let Some(sandbox) = &virtiofsd.sandbox {
        command.args(["--sandbox", sandbox]);
    }

    let mut child = command.spawn().map_err(|_| VirtioFsStartError::Failed)?;

    for _ in 0..VIRTIOFSD_STARTUP_ATTEMPTS {
        if signal_value(signal).is_some() {
            stop_child(&mut child);
            let _ = fs::remove_file(&socket);
            return Err(VirtioFsStartError::Interrupted);
        }

        match child.try_wait() {
            Ok(Some(_)) => {
                let _ = fs::remove_file(&socket);
                return Err(VirtioFsStartError::Failed);
            }
            Ok(None) => {}
            Err(_) => {
                stop_child(&mut child);
                let _ = fs::remove_file(&socket);
                return Err(VirtioFsStartError::Failed);
            }
        }
        if socket
            .metadata()
            .is_ok_and(|metadata| metadata.file_type().is_socket())
        {
            return Ok(VirtioFsProcess { child, socket });
        }
        std::thread::sleep(Duration::from_millis(100));
    }

    stop_child(&mut child);
    let _ = fs::remove_file(&socket);
    Err(VirtioFsStartError::Failed)
}

fn resolve_path(work_dir: &Path, path: PathBuf) -> PathBuf {
    if path.is_absolute() {
        path
    } else {
        work_dir.join(path)
    }
}

fn ensure_directory_tree(path: &Path) -> Result<(), VirtioFsStartError> {
    if path.as_os_str().is_empty() {
        return Err(VirtioFsStartError::Failed);
    }

    let mut current = PathBuf::new();
    for component in path.components() {
        current.push(component.as_os_str());
        match fs::symlink_metadata(&current) {
            Ok(metadata) if metadata.file_type().is_symlink() => {
                return Err(VirtioFsStartError::Failed);
            }
            Ok(metadata) if !metadata.is_dir() => {
                return Err(VirtioFsStartError::Failed);
            }
            Ok(_) => {}
            Err(err) if err.kind() == std::io::ErrorKind::NotFound => {
                if let Err(err) = fs::create_dir(&current)
                    && err.kind() != std::io::ErrorKind::AlreadyExists
                {
                    return Err(VirtioFsStartError::Failed);
                }
                let metadata =
                    fs::symlink_metadata(&current).map_err(|_| VirtioFsStartError::Failed)?;
                if metadata.file_type().is_symlink() {
                    return Err(VirtioFsStartError::Failed);
                }
                if !metadata.is_dir() {
                    return Err(VirtioFsStartError::Failed);
                }
            }
            Err(_) => {
                return Err(VirtioFsStartError::Failed);
            }
        }
    }
    Ok(())
}

fn stop_virtiofsd(process: &mut VirtioFsProcess) {
    stop_child(&mut process.child);
    let _ = fs::remove_file(&process.socket);
}

fn stop_child(child: &mut Child) {
    if child.try_wait().ok().flatten().is_none() {
        let _ = child.kill();
    }
    let _ = child.wait();
}
