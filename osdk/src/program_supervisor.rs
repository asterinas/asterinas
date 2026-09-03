// SPDX-License-Identifier: MPL-2.0

//! Supervises running host programs for the duration of QEMU.

use std::{
    path::Path,
    process::{Child, ExitStatus},
    sync::atomic::AtomicI32,
    time::Duration,
};

use crate::{
    error::Errno,
    error_msg,
    program::{POLL_INTERVAL, RunningProgram},
    signal::signal_value,
    warn_msg,
};

/// Delay between starting managed programs and continuing QEMU setup.
const STARTUP_DELAY: Duration = Duration::from_secs(1);

/// Supervises host programs for the duration of a QEMU run.
///
/// The supervisor starts all executables, reaps normal exits, reports failures, and
/// stops all running programs when QEMU exits or the run is interrupted.
pub(crate) struct ProgramSupervisor<'a> {
    /// Host programs currently managed by the supervisor.
    running_programs: Vec<RunningProgram>,
    /// Shared state used to observe termination signals.
    signal: &'a AtomicI32,
    /// Working directory assigned to each managed host program.
    work_dir: &'a Path,
}

impl<'a> ProgramSupervisor<'a> {
    /// Starts all program executables.
    pub(crate) fn start(
        executables: &[String],
        work_dir: &'a Path,
        signal: &'a AtomicI32,
    ) -> Result<Self, Errno> {
        let mut supervisor = Self {
            running_programs: Vec::new(),
            signal,
            work_dir,
        };

        for executable in executables {
            if signal_value(signal).is_some() {
                supervisor.stop_all();
                return Err(Errno::Interrupted);
            }

            if let Err(errno) = supervisor.start_program(executable) {
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

    /// Starts one program executable and records its running state.
    fn start_program(&mut self, executable: &str) -> Result<(), Errno> {
        info!("Starting managed QEMU program `{executable}`");

        let running = RunningProgram::new(executable, self.work_dir)?;
        self.running_programs.push(running);

        Ok(())
    }

    /// Waits for QEMU while observing managed programs and termination signals.
    pub(crate) fn wait_for_qemu(&mut self, qemu: &mut Child) -> Result<ExitStatus, Errno> {
        loop {
            if signal_value(self.signal).is_some() {
                stop_qemu(qemu);
                self.stop_all();
                return Err(Errno::Interrupted);
            }

            let status = match qemu.try_wait() {
                Ok(status) => status,
                Err(err) => {
                    warn_msg!("failed to poll QEMU: {err}");

                    stop_qemu(qemu);
                    self.stop_all();

                    return Err(Errno::ExecuteCommand);
                }
            };

            if let Some(status) = status {
                if let Err(errno) = self.check_children() {
                    stop_qemu(qemu);
                    return Err(errno);
                }
                return Ok(status);
            }

            if let Err(errno) = self.check_children() {
                stop_qemu(qemu);
                return Err(errno);
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    }

    /// Reaps normally exited programs and reports abnormal exits.
    pub(crate) fn check_children(&mut self) -> Result<(), Errno> {
        let mut index = 0;

        while index < self.running_programs.len() {
            let status = self.running_programs[index].try_wait().map_err(|err| {
                warn_msg!(
                    "failed to poll managed QEMU program `{}`: {err}",
                    self.running_programs[index].executable()
                );
                Errno::ExecuteCommand
            })?;

            let Some(status) = status else {
                index += 1;
                continue;
            };

            let executable = self.running_programs[index].executable().to_owned();
            if status.success() {
                self.running_programs.remove(index);
                continue;
            }

            error_msg!(
                "managed QEMU program `{executable}` exited unexpectedly with status {status}"
            );

            self.stop_all();

            return Err(Errno::ExecuteCommand);
        }

        Ok(())
    }

    /// Stops all managed programs and releases their runtime state.
    pub(crate) fn stop_all(&mut self) {
        for running in &mut self.running_programs {
            running.stop();
        }

        self.running_programs.clear();
    }
}

impl Drop for ProgramSupervisor<'_> {
    fn drop(&mut self) {
        self.stop_all();
    }
}

pub(crate) fn stop_qemu(qemu: &mut Child) {
    let _ = qemu.kill();
    let _ = qemu.wait();
}
