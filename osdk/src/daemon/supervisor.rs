// SPDX-License-Identifier: MPL-2.0

//! Supervises running host daemons for the duration of QEMU.

use std::{
    path::Path,
    process::{self, Child, ExitStatus},
    time::{Duration, Instant},
};

use crate::{error::Errno, error_msg, warn_msg};

use super::{DaemonScheme, POLL_INTERVAL, RunningDaemon, SHUTDOWN_TIMEOUT, SignalGuard};

/// Time given to a daemon to create resources needed by QEMU before setup
/// continues. QEMU socket backends fail if their socket does not exist yet.
const STARTUP_DELAY: Duration = Duration::from_secs(1);

/// Supervises host daemons for the duration of a QEMU run.
///
/// The supervisor starts all executables, reaps normal exits, reports failures, and
/// stops all running daemons when QEMU exits or the run is interrupted.
pub(crate) struct DaemonSupervisor<'a> {
    /// Host daemons currently managed by the supervisor.
    running_daemons: Vec<RunningDaemon<'a>>,
    /// Shared state used to observe termination signals.
    signal: &'a SignalGuard,
    /// Working directory assigned to each managed host daemon.
    work_dir: &'a Path,
}

impl<'a> DaemonSupervisor<'a> {
    /// Starts all daemon executables.
    pub(crate) fn start(
        daemons: &'a [DaemonScheme],
        work_dir: &'a Path,
        signal: &'a SignalGuard,
    ) -> Result<Self, Errno> {
        let mut supervisor = Self {
            running_daemons: Vec::new(),
            signal,
            work_dir,
        };

        for daemon in daemons {
            if signal.received_signal().is_some() {
                supervisor.stop_all();
                return Err(Errno::Interrupted);
            }

            if let Err(errno) = supervisor.start_daemon(daemon) {
                supervisor.stop_all();
                return Err(errno);
            }

            // FIXME: Replace this fixed delay with a readiness check for the
            // resources that QEMU needs.
            std::thread::sleep(STARTUP_DELAY);

            if signal.received_signal().is_some() {
                supervisor.stop_all();
                return Err(Errno::Interrupted);
            }

            if let Err(errno) = supervisor.reap_exited_daemons() {
                supervisor.stop_all();
                return Err(errno);
            }
        }

        Ok(supervisor)
    }

    /// Starts one daemon executable and records its running state.
    fn start_daemon(&mut self, daemon: &'a DaemonScheme) -> Result<(), Errno> {
        info!("Starting managed QEMU daemon `{}`", daemon.path.display());

        let running = RunningDaemon::new(daemon, self.work_dir)?;
        self.running_daemons.push(running);

        Ok(())
    }

    /// Waits for QEMU while observing managed daemons and termination signals.
    pub(crate) fn wait_for_qemu(&mut self, qemu: &mut Child) -> Result<ExitStatus, Errno> {
        loop {
            if self.signal.received_signal().is_some() {
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
                // Daemons are expected to terminate with their QEMU peer, so
                // their exit status is no longer meaningful here.
                self.stop_all();
                return Ok(status);
            }

            if let Err(errno) = self.reap_exited_daemons() {
                self.stop_qemu_and_daemons(qemu);
                return Err(errno);
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Reaps normally exited daemons and reports abnormal exits.
    pub(crate) fn reap_exited_daemons(&mut self) -> Result<(), Errno> {
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
                info!("managed QEMU daemon `{executable}` exited successfully");
                self.running_daemons.remove(index);
                continue;
            }

            error_msg!(
                "managed QEMU daemon `{executable}` exited unexpectedly with status {status}"
            );

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

    /// Stops QEMU when present, stops all managed daemons, and exits.
    pub(crate) fn abort(&mut self, qemu: Option<&mut Child>, errno: Errno) -> ! {
        if let Some(qemu) = qemu {
            stop_qemu(qemu);
        }
        self.stop_all();
        process::exit(errno as _);
    }
}

impl Drop for DaemonSupervisor<'_> {
    // `process::exit` does not run destructors, so error paths that exit the
    // process must stop daemons explicitly. This covers ordinary returns and
    // panics.
    fn drop(&mut self) {
        self.stop_all();
    }
}

fn stop_qemu(qemu: &mut Child) {
    let _ = qemu.kill();
    let _ = qemu.wait();
}

#[cfg(test)]
mod tests {
    use std::{fs, path::Path};

    use super::*;
    use tempfile::tempdir;

    fn shell_daemon(script: &str) -> DaemonScheme {
        DaemonScheme {
            path: "/bin/sh".into(),
            args: vec!["-c".into(), script.into()],
        }
    }

    #[test]
    fn nonzero_daemon_exit_aborts_startup() {
        let signal_guard = SignalGuard::install().unwrap();
        let daemon = shell_daemon("exit 1");
        let daemons = [daemon];

        let result = DaemonSupervisor::start(&daemons, Path::new("."), &signal_guard);

        assert!(matches!(result, Err(Errno::ExecuteCommand)));
    }

    #[test]
    fn successful_daemon_exit_is_reaped() {
        let signal_guard = SignalGuard::install().unwrap();
        let daemon = shell_daemon("exit 0");
        let daemons = [daemon];

        let supervisor = DaemonSupervisor::start(&daemons, Path::new("."), &signal_guard)
            .expect("successful daemon exit should not abort startup");

        assert!(supervisor.running_daemons.is_empty());
    }

    #[test]
    fn stop_all_terminates_running_daemon() {
        let signal_guard = SignalGuard::install().unwrap();
        let daemon = shell_daemon("sleep 30");
        let daemons = [daemon];
        let mut supervisor = DaemonSupervisor::start(&daemons, Path::new("."), &signal_guard)
            .expect("running daemon should start successfully");

        assert!(!supervisor.running_daemons.is_empty());
        supervisor.stop_all();
        assert!(supervisor.running_daemons.is_empty());
    }

    #[test]
    fn stop_all_escalates_when_daemon_ignores_sigterm() {
        let signal_guard = SignalGuard::install().unwrap();
        let daemon = shell_daemon("trap '' TERM; sleep 30");
        let daemons = [daemon];
        let mut supervisor = DaemonSupervisor::start(&daemons, Path::new("."), &signal_guard)
            .expect("daemon should start successfully");

        supervisor.stop_all();
        assert!(supervisor.running_daemons.is_empty());
    }

    #[test]
    fn stop_all_terminates_daemon_descendants() {
        let signal_guard = SignalGuard::install().unwrap();
        let work_dir = tempdir().unwrap();
        let daemon = shell_daemon("sleep 30 & echo $! > child.pid; wait");
        let daemons = [daemon];
        let mut supervisor = DaemonSupervisor::start(&daemons, work_dir.path(), &signal_guard)
            .expect("daemon should start successfully");

        let child_pid: i32 = fs::read_to_string(work_dir.path().join("child.pid"))
            .unwrap()
            .trim()
            .parse()
            .unwrap();
        supervisor.stop_all();

        assert!(!process_exists(child_pid));
    }

    fn process_exists(pid: i32) -> bool {
        let path = format!("/proc/{pid}/stat");
        let Ok(stat) = fs::read_to_string(path) else {
            return false;
        };
        !stat
            .split_whitespace()
            .nth(2)
            .is_some_and(|state| state == "Z")
    }
}
