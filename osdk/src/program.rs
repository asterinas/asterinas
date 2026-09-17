// SPDX-License-Identifier: MPL-2.0

//! Configuration and lifecycle operations for host daemons managed during QEMU.

use std::{
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    time::Duration,
};

use rustix::process::{self, Pid, Signal};

use crate::{error::Errno, error_msg};

/// Interval between polls while monitoring QEMU and shutting down daemons.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Maximum time allowed for a daemon to exit after `SIGTERM`.
pub(crate) const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// A host daemon that has been started and is supervised until QEMU exits.
///
/// The owning supervisor polls the child until it exits and reaps it. When the
/// QEMU run ends or is interrupted, the supervisor terminates the daemon and
/// reaps its child process.
pub(crate) struct RunningDaemon {
    executable: PathBuf,
    child: Child,
}

impl RunningDaemon {
    pub(crate) fn new(
        executable: impl AsRef<Path>,
        work_dir: impl AsRef<Path>,
    ) -> Result<Self, Errno> {
        let executable = executable.as_ref();
        let mut command = Command::new(executable);

        command.current_dir(work_dir.as_ref());
        // Each managed daemon is started as the leader of its own process group
        // This allows descendants to be terminated in a clean boundary.
        command.process_group(0);

        let child = command.spawn().map_err(|err| {
            error_msg!("failed to spawn `{}`: {err}", executable.display());
            Errno::ExecuteCommand
        })?;

        Ok(Self {
            executable: executable.to_owned(),
            child,
        })
    }

    pub(crate) fn executable(&self) -> &Path {
        &self.executable
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn kill_and_wait(&mut self) {
        self.kill();
        let _ = self.child.wait();
    }

    pub(crate) fn kill(&mut self) {
        send_group_signal(&self.child, Signal::KILL);
        let _ = self.child.kill();
    }

    pub(crate) fn terminate(&self) {
        send_group_signal(&self.child, Signal::TERM);
    }
}

fn send_group_signal(child: &Child, signal: Signal) {
    let _ = process::kill_process_group(Pid::from_child(child), signal);
}
