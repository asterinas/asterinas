// SPDX-License-Identifier: MPL-2.0

//! Configuration and lifecycle operations for host programs managed during QEMU.

use std::{
    os::unix::process::CommandExt,
    path::Path,
    process::{Child, Command, ExitStatus},
    time::{Duration, Instant},
};

use rustix::process::{self, Pid, Signal};

use crate::{error::Errno, error_msg, warn_msg};

/// Interval between polls while monitoring QEMU and shutting down programs.
pub(crate) const POLL_INTERVAL: Duration = Duration::from_millis(50);
/// Maximum time allowed for a program to exit after `SIGTERM`.
const SHUTDOWN_TIMEOUT: Duration = Duration::from_secs(5);

/// A host program that has been started and is supervised until QEMU exits.
pub(crate) struct RunningProgram {
    executable: String,
    child: Child,
}

impl RunningProgram {
    pub(crate) fn new(executable: &str, work_dir: &Path) -> Result<Self, Errno> {
        let executable = expand_env_template(executable)?;
        let mut command = Command::new(&executable);

        command.current_dir(work_dir);
        command.process_group(0);

        let child = command.spawn().map_err(|err| {
            error_msg!("failed to spawn `{executable}`: {err}");
            Errno::ExecuteCommand
        })?;

        Ok(Self {
            executable: executable.to_owned(),
            child,
        })
    }

    pub(crate) fn executable(&self) -> &str {
        &self.executable
    }

    pub(crate) fn try_wait(&mut self) -> std::io::Result<Option<ExitStatus>> {
        self.child.try_wait()
    }

    pub(crate) fn stop(&mut self) {
        send_group_signal(&self.child, Signal::TERM);

        match self.child.try_wait() {
            Ok(Some(_)) => return,
            Err(err) => {
                warn_msg!(
                    "failed to poll managed QEMU program `{}`: {err}",
                    self.executable()
                );

                send_group_signal(&self.child, Signal::KILL);
                let _ = self.child.kill();
                let _ = self.child.wait();

                return;
            }
            Ok(None) => {}
        }

        let deadline = Instant::now() + SHUTDOWN_TIMEOUT;

        loop {
            match self.child.try_wait() {
                Ok(Some(_)) => return,
                Ok(None) => {
                    if Instant::now() < deadline {
                        std::thread::sleep(POLL_INTERVAL);
                        continue;
                    }

                    warn_msg!(
                        "managed QEMU program `{}` did not stop in time; killing it",
                        self.executable()
                    );

                    send_group_signal(&self.child, Signal::KILL);

                    let _ = self.child.kill();
                    let _ = self.child.wait();

                    return;
                }
                Err(err) => {
                    warn_msg!(
                        "failed to reap managed program `{}`: {err}",
                        self.executable()
                    );

                    send_group_signal(&self.child, Signal::KILL);

                    let _ = self.child.kill();
                    let _ = self.child.wait();

                    return;
                }
            }
        }
    }
}

pub(crate) fn expand_env_template(value: &str) -> Result<String, Errno> {
    let mut output = String::with_capacity(value.len());
    let mut rest = value;

    while let Some(start) = rest.find("${") {
        output.push_str(&rest[..start]);

        let expression = &rest[start + 2..];

        let Some(end) = expression.find('}') else {
            error_msg!("unterminated environment variable in `{value}`");
            return Err(Errno::ParseMetadata);
        };

        let name = &expression[..end];

        if name.is_empty() {
            error_msg!("empty environment variable in `{value}`");
            return Err(Errno::ParseMetadata);
        }

        let value = std::env::var(name).map_err(|_| {
            error_msg!("environment variable `{name}` is not set");
            Errno::ParseMetadata
        })?;

        output.push_str(&value);

        rest = &expression[end + 1..];
    }

    output.push_str(rest);

    Ok(output)
}

fn send_group_signal(child: &Child, signal: Signal) {
    let _ = process::kill_process_group(Pid::from_child(child), signal);
}

#[cfg(test)]
mod tests {
    use super::expand_env_template;

    #[test]
    fn expands_environment_templates_without_shell_parsing() {
        let path = std::env::var("PATH").unwrap();

        assert_eq!(
            expand_env_template("prefix-${PATH}-suffix").unwrap(),
            format!("prefix-{path}-suffix")
        );
        assert_eq!(expand_env_template("literal").unwrap(), "literal");
        assert!(expand_env_template("${__OSDK_MISSING_VARIABLE__}").is_err());
        assert!(expand_env_template("${PATH").is_err());
    }
}
