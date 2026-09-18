// SPDX-License-Identifier: MPL-2.0

//! Supervises running host daemons for the duration of QEMU.

use std::{
    path::Path,
    process::{Child, ExitStatus},
    sync::atomic::AtomicI32,
    time::{Duration, Instant},
};

use crate::{error::Errno, error_msg, warn_msg};

use super::{Daemon, POLL_INTERVAL, RunningDaemon, SHUTDOWN_TIMEOUT, signal_value};

/// Delay between starting managed daemons and continuing QEMU setup.
const STARTUP_DELAY: Duration = Duration::from_secs(1);

/// Supervises host daemons for the duration of a QEMU run.
///
/// The supervisor starts all executables, reaps normal exits, reports failures, and
/// stops all running daemons when QEMU exits or the run is interrupted.
pub(crate) struct DaemonSupervisor<'a> {
    /// Host daemons currently managed by the supervisor.
    running_daemons: Vec<RunningDaemon<'a>>,
    /// Shared state used to observe termination signals.
    signal: &'a AtomicI32,
    /// Working directory assigned to each managed host daemon.
    work_dir: &'a Path,
}

impl<'a> DaemonSupervisor<'a> {
    /// Starts all daemon executables.
    pub(crate) fn start(
        daemons: &'a [Daemon],
        work_dir: &'a Path,
        signal: &'a AtomicI32,
    ) -> Result<Self, Errno> {
        let mut supervisor = Self {
            running_daemons: Vec::new(),
            signal,
            work_dir,
        };

        for daemon in daemons {
            if signal_value(signal).is_some() {
                supervisor.stop_all();
                return Err(Errno::Interrupted);
            }

            if let Err(errno) = supervisor.start_daemon(daemon) {
                supervisor.stop_all();
                return Err(errno);
            }

            std::thread::sleep(STARTUP_DELAY);

            if signal_value(signal).is_some() {
                supervisor.stop_all();
                return Err(Errno::Interrupted);
            }

            if let Err(errno) = supervisor.check_children() {
                supervisor.stop_all();
                return Err(errno);
            }
        }

        Ok(supervisor)
    }

    /// Starts one daemon executable and records its running state.
    fn start_daemon(&mut self, daemon: &'a Daemon) -> Result<(), Errno> {
        info!("Starting managed QEMU daemon `{}`", daemon.path.display());

        let running = daemon.run(self.work_dir)?;
        self.running_daemons.push(running);

        Ok(())
    }

    /// Waits for QEMU while observing managed daemons and termination signals.
    pub(crate) fn wait_for_qemu(&mut self, qemu: &mut Child) -> Result<ExitStatus, Errno> {
        loop {
            if signal_value(self.signal).is_some() {
                self.stop_qemu_and_daemons(qemu);
                return Err(Errno::Interrupted);
            }

            let status = match qemu.try_wait() {
                Ok(status) => status,
                Err(err) => {
                    warn_msg!("failed to poll QEMU: {err}");

                    self.stop_qemu_and_daemons(qemu);

                    return Err(Errno::ExecuteCommand);
                }
            };

            if let Some(status) = status {
                if let Err(errno) = self.check_children() {
                    self.stop_qemu_and_daemons(qemu);
                    return Err(errno);
                }
                return Ok(status);
            }

            if let Err(errno) = self.check_children() {
                self.stop_qemu_and_daemons(qemu);
                return Err(errno);
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Reaps normally exited daemons and reports abnormal exits.
    pub(crate) fn check_children(&mut self) -> Result<(), Errno> {
        let mut index = 0;

        while index < self.running_daemons.len() {
            let status = self.running_daemons[index].try_wait().map_err(|err| {
                warn_msg!(
                    "failed to poll managed QEMU daemon `{}`: {err}",
                    self.running_daemons[index].executable().display()
                );
                Errno::ExecuteCommand
            })?;

            let Some(status) = status else {
                index += 1;
                continue;
            };

            let executable = self.running_daemons[index]
                .executable()
                .display()
                .to_string();
            if status.success() {
                self.running_daemons.remove(index);
                continue;
            }

            error_msg!(
                "managed QEMU daemon `{executable}` exited unexpectedly with status {status}"
            );

            self.stop_all();

            return Err(Errno::ExecuteCommand);
        }

        Ok(())
    }

    /// Stops all managed daemons and releases their runtime state.
    pub(crate) fn stop_all(&mut self) {
        for running in &self.running_daemons {
            running.terminate();
        }

        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;
        while Instant::now() < deadline {
            let mut any_running = false;

            for running in &mut self.running_daemons {
                match running.try_wait() {
                    Ok(Some(_)) => {}
                    Ok(None) => any_running = true,
                    Err(err) => {
                        warn_msg!(
                            "failed to poll managed QEMU daemon `{}`: {err}",
                            running.executable().display()
                        );
                        running.kill();
                    }
                }
            }

            if !any_running {
                break;
            }
            std::thread::sleep(POLL_INTERVAL);
        }

        for running in &mut self.running_daemons {
            match running.try_wait() {
                Ok(Some(_)) => {}
                Ok(None) => {
                    warn_msg!(
                        "managed QEMU daemon `{}` did not stop in time; killing it",
                        running.executable().display()
                    );
                    running.kill_and_wait();
                }
                Err(err) => {
                    warn_msg!(
                        "failed to reap managed daemon `{}`: {err}; killing it",
                        running.executable().display()
                    );
                    running.kill_and_wait();
                }
            }
        }

        self.running_daemons.clear();
    }

    /// Stops QEMU and all managed daemons.
    pub(crate) fn stop_qemu_and_daemons(&mut self, qemu: &mut Child) {
        stop_qemu(qemu);
        self.stop_all();
    }
}

impl Drop for DaemonSupervisor<'_> {
    fn drop(&mut self) {
        self.stop_all();
    }
}

fn stop_qemu(qemu: &mut Child) {
    let _ = qemu.kill();
    let _ = qemu.wait();
}
