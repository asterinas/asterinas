// SPDX-License-Identifier: MPL-2.0

//! Configuration and lifecycle operations for host daemons managed during QEMU.

mod signal;
mod supervisor;

use std::{
    os::unix::process::CommandExt,
    path::{Path, PathBuf},
    process::{Child, Command, ExitStatus},
    time::Duration,
};

use rustix::process::{self, Pid, Signal};

use crate::{error::Errno, error_msg};

pub(crate) use signal::{SignalGuard, signal_value};
pub(crate) use supervisor::DaemonSupervisor;

/// Interval between polls while monitoring QEMU and shutting down daemons.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Maximum time allowed for a daemon to exit after `SIGTERM`.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

#[derive(Clone, Debug, Deserialize, Eq, PartialEq, Serialize)]
pub(crate) struct Daemon {
    pub(crate) path: PathBuf,
    #[serde(default)]
    pub(crate) args: Vec<String>,
}

impl Daemon {
    fn run(&self, work_dir: &Path) -> Result<RunningDaemon<'_>, Errno> {
        let mut command = Command::new(&self.path);
        command.args(&self.args);

        command.current_dir(work_dir);
        // Each managed daemon is started as the leader of its own process group
        // This allows descendants to be terminated in a clean boundary.
        command.process_group(0);

        let child = command.spawn().map_err(|err| {
            error_msg!("failed to spawn `{}`: {err}", self.path.display());
            Errno::ExecuteCommand
        })?;

        Ok(RunningDaemon {
            daemon: self,
            child,
        })
    }
}

/// A host daemon that has been started and is supervised until QEMU exits.
///
/// The owning supervisor polls the child until it exits and reaps it. When the
/// QEMU run ends or is interrupted, the supervisor terminates the daemon and
/// reaps its child process.
struct RunningDaemon<'a> {
    daemon: &'a Daemon,
    child: Child,
}

impl RunningDaemon<'_> {
    fn executable(&self) -> &Path {
        &self.daemon.path
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    fn kill_and_wait(&mut self) {
        self.kill();
        let _ = self.child.wait();
    }

    fn kill(&mut self) {
        let _ = process::kill_process_group(Pid::from_child(&self.child), Signal::KILL);
    }

    fn terminate(&self) {
        let _ = process::kill_process_group(Pid::from_child(&self.child), Signal::TERM);
    }
}
