// SPDX-License-Identifier: MPL-2.0

//! Lifecycle management for host daemons running alongside QEMU.

mod signal;
mod supervisor;

use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, ExitStatus},
    time::Duration,
};

use rustix::process::{self, Pid, Signal};

use crate::{config::scheme::DaemonScheme, error::Errno, error_msg};

pub(crate) use signal::SignalGuard;
pub(crate) use supervisor::DaemonSupervisor;

/// Interval between polls while monitoring QEMU and shutting down daemons.
const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Maximum time allowed for a daemon to exit after `SIGTERM`.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// A host daemon that has been started and is supervised until QEMU exits.
///
/// The owning supervisor polls the child until it exits and reaps it. When the
/// QEMU run ends or is interrupted, the supervisor terminates the daemon and
/// reaps its child process.
struct RunningDaemon<'a> {
    daemon: &'a DaemonScheme,
    child: Child,
    reaped: bool,
}

impl<'a> RunningDaemon<'a> {
    fn new(daemon: &'a DaemonScheme, work_dir: &Path) -> Result<Self, Errno> {
        let mut command = Command::new(&daemon.path);
        command.args(&daemon.args);

        command.current_dir(work_dir);
        // Each managed daemon is started as the leader of its own process group.
        // This allows descendants to be terminated in a clean boundary.
        command.process_group(0);

        let child = command.spawn().map_err(|err| {
            error_msg!("failed to spawn `{}`: {err}", daemon.path.display());
            Errno::ExecuteCommand
        })?;

        Ok(Self {
            daemon,
            child,
            reaped: false,
        })
    }

    fn executable(&self) -> &Path {
        &self.daemon.path
    }

    fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        let status = self.child.try_wait()?;
        self.reaped |= status.is_some();
        Ok(status)
    }

    fn kill_and_wait(&mut self) {
        self.kill();
        let _ = self.child.wait();
    }

    fn kill(&mut self) {
        if self.reaped {
            return;
        }
        let _ = process::kill_process_group(Pid::from_child(&self.child), Signal::KILL);
    }

    fn terminate(&self) {
        if self.reaped {
            return;
        }
        let _ = process::kill_process_group(Pid::from_child(&self.child), Signal::TERM);
    }
}
