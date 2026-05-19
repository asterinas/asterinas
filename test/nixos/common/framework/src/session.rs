// SPDX-License-Identifier: MPL-2.0

use std::{
    borrow::Cow,
    time::{Duration, Instant},
};

use rexpect::{reader::Regex, session::PtySession};
use uuid::Uuid;

use super::{Error, clean_output, truncate_output_for_error};

/// Describes a runtime session configuration.
///
/// A session descriptor defines how to interact with a particular execution context,
/// such as a shell environment, container, or remote session. It specifies the prompt
/// pattern and the commands needed to enter and exit the context.
///
/// # Examples
///
/// ```rust
/// use nixos_test_framework::SessionDesc;
///
/// let desc = SessionDesc::new("/ #", "podman run -it alpine", "exit");
/// ```
pub struct SessionDesc {
    prompt: Cow<'static, str>,
    enter_cmd: Cow<'static, str>,
    exit_cmd: Cow<'static, str>,
}

impl SessionDesc {
    /// Creates a new session descriptor.
    pub fn new(
        prompt: impl Into<Cow<'static, str>>,
        enter_cmd: impl Into<Cow<'static, str>>,
        exit_cmd: impl Into<Cow<'static, str>>,
    ) -> Self {
        Self {
            prompt: prompt.into(),
            enter_cmd: enter_cmd.into(),
            exit_cmd: exit_cmd.into(),
        }
    }
}

/// An interactive session for running commands in a test environment.
///
/// `Session` provides a high-level interface for interacting with the test environment.
/// It manages execution contexts and handles nested environments automatically.
///
/// It uses a [`SessionDesc`] to track the current prompt and the correct command
/// to exit the current context.
pub struct Session {
    uuid: String,
    desc: SessionDesc,
    pty_session: PtySession,
}

impl Session {
    /// Creates a new session with the given PTY session and descriptor.
    pub(super) fn new(desc: SessionDesc, pty_session: PtySession) -> Self {
        Self {
            uuid: Uuid::new_v4().to_string(),
            desc,
            pty_session,
        }
    }

    /// Executes a command in the current session.
    ///
    /// This method runs the specified command and waits for it to complete.
    /// The command is considered complete when the session prompt reappears.
    ///
    /// Returns an error if the command times out or the session terminates unexpectedly.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use nixos_test_framework::*;
    ///
    /// fn example(nixos_shell: &mut Session) -> Result<(), Error> {
    ///     // Execute simple commands
    ///     nixos_shell.run_cmd("ls -la")?;
    ///     nixos_shell.run_cmd("cd /tmp")?;
    ///     nixos_shell.run_cmd("mkdir test_dir")?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn run_cmd(&mut self, command: &str) -> Result<(), Error> {
        println!("--> Running: {}", command);
        self.pty_session.send_line(command).map_err(Error::from)?;
        // Read and consume the echoed command line
        self.pty_session.exp_string(command).map_err(Error::from)?;

        if let Err(error) = self
            .pty_session
            .exp_string(&self.desc.prompt)
            .map_err(Error::from)
        {
            Self::output_error(&error);
            return Err(error);
        }

        Ok(())
    }

    /// Executes a command and verifies its output contains expected text.
    ///
    /// This method runs the command and checks that the specified string appears
    /// in the output. This is useful for validating command results.
    ///
    /// Returns an error if:
    /// - The expected string is not found in the output
    /// - The command times out
    /// - The session terminates unexpectedly
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use nixos_test_framework::*;
    ///
    /// fn example(nixos_shell: &mut Session) -> Result<(), Error> {
    ///     // Verify output contains expected string
    ///     nixos_shell.run_cmd_and_expect("echo 'Hello, World!'", "Hello")?;
    ///
    ///     // Check if a file exists
    ///     nixos_shell.run_cmd_and_expect("ls /etc/hostname", "/etc/hostname")?;
    ///
    ///     // Verify system information
    ///     nixos_shell.run_cmd_and_expect("cat /etc/os-release", "NixOS")?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn run_cmd_and_expect(&mut self, command: &str, expected: &str) -> Result<(), Error> {
        println!("--> Running: {} (expecting: {})", command, expected);
        self.pty_session.send_line(command).map_err(Error::from)?;
        // Read and consume the echoed command line
        self.pty_session.exp_string(command).map_err(Error::from)?;

        match self
            .pty_session
            .exp_string(&self.desc.prompt)
            .map_err(Error::from)
        {
            Ok(unread) => {
                let cleaned_unread = clean_output(&unread);
                if !cleaned_unread.contains(expected) {
                    let error = Error::UnexpectedOutput {
                        expected: expected.to_string(),
                        got: truncate_output_for_error(&cleaned_unread),
                    };
                    Self::output_error(&error);
                    return Err(error);
                }
            }
            Err(e) => {
                Self::output_error(&e);
                return Err(e);
            }
        }

        Ok(())
    }

    /// Executes a command and verifies its output matches the expected regex.
    ///
    /// This method runs the command and checks that the specified regex matches
    /// the output before the prompt reappears.
    ///
    /// Returns an error if:
    /// - The expected regex does not match the output
    /// - The command times out
    /// - The session terminates unexpectedly
    ///
    /// # Example
    ///
    /// ```rust,norun
    /// use nixos_test_framework::*;
    /// use rexpect::reader::Regex;
    ///
    /// fn example(nixos_shell: &mut Session) -> Result<(), Error> {
    ///     let expected = Regex::new(r"(?m)^Hello, .*!$").unwrap();
    ///     nixos_shell.run_cmd_and_expect_regex("echo 'Hello, World!'", &expected)?;
    ///     Ok(())
    /// }
    /// ```
    pub fn run_cmd_and_expect_regex(
        &mut self,
        command: &str,
        expected: &Regex,
    ) -> Result<(), Error> {
        let command_output = self.run_cmd_and_collect_output(command)?;

        if command_output.exit_status != 0 {
            return Err(Error::NonZeroExit {
                exit_status: command_output.exit_status,
                output: truncate_output_for_error(&command_output.output),
            });
        }

        if !expected.is_match(&command_output.output) {
            let error = Error::UnexpectedOutput {
                expected: expected.as_str().to_string(),
                got: truncate_output_for_error(&command_output.output),
            };
            Self::output_error(&error);
            return Err(error);
        }

        Ok(())
    }

    /// Enters a nested session, runs operations, and automatically exits.
    ///
    /// This method is used to work with nested environments like containers, SSH sessions,
    /// or any interactive shell.
    ///
    /// Returns an error if entering, running operations, or exiting fails.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use nixos_test_framework::*;
    ///
    /// fn container_test(nixos_shell: &mut Session) -> Result<(), Error> {
    ///     // Define the container session
    ///     let container_session_desc = SessionDesc::new(
    ///         "/ #",
    ///         "podman run -it docker.io/library/alpine",
    ///         "exit",
    ///     );
    ///
    ///     // Enter container, run tests, and automatically exit
    ///     nixos_shell.enter_session_and_run(container_session_desc, |alpine_shell| {
    ///         alpine_shell.run_cmd_and_expect(
    ///             "cat /etc/os-release",
    ///             "Alpine"
    ///         )?;
    ///         Ok(())
    ///     })?;
    ///
    ///     // Back in the host - container has been exited
    ///     nixos_shell.run_cmd("echo 'Back on host'")?;
    ///     Ok(())
    /// }
    /// ```
    pub fn enter_session_and_run<F>(&mut self, desc: SessionDesc, test_ops: F) -> Result<(), Error>
    where
        F: FnOnce(&mut Session) -> Result<(), Error>,
    {
        let old_desc = std::mem::replace(&mut self.desc, desc);

        if let Err(e) = self.run_cmd(&self.desc.enter_cmd.clone()) {
            self.desc = old_desc;
            return Err(e);
        }

        let res = test_ops(self);

        let exit_cmd = self.desc.exit_cmd.clone();
        self.desc = old_desc;
        let exit_res = self.run_cmd(&exit_cmd);

        res?;
        exit_res?;

        Ok(())
    }

    /// Starts a background process, waits for readiness, and runs test operations.
    ///
    /// This method is intended for test cases that need temporary daemons such as HTTP servers.
    /// It polls until the process becomes ready instead of relying on fixed sleeps. When the
    /// method returns or unwinds, it makes a best-effort attempt to stop the background process.
    ///
    /// # Examples
    ///
    /// ```rust,no_run
    /// use nixos_test_framework::*;
    ///
    /// fn example(nixos_shell: &mut Session) -> Result<(), Error> {
    ///     nixos_shell.with_background_process(
    ///         BackgroundProcess::new(
    ///             "python3 -m http.server 8000 >/tmp/http.log 2>&1 &",
    ///             CommandCheck::new("curl http://127.0.0.1:8000", "Directory listing"),
    ///             "pkill -f 'python3 -m http.server 8000'",
    ///             CommandCheck::new(
    ///                 "! pgrep -f 'python3 -m http.server 8000' >/dev/null && echo stopped",
    ///                 "stopped",
    ///             ),
    ///         ),
    ///         |shell| shell.run_cmd_and_expect("curl http://127.0.0.1:8000", "Directory listing"),
    ///     )?;
    ///
    ///     Ok(())
    /// }
    /// ```
    pub fn with_background_process<F>(
        &mut self,
        background_process: BackgroundProcess,
        test_ops: F,
    ) -> Result<(), Error>
    where
        F: FnOnce(&mut Session) -> Result<(), Error>,
    {
        let guard = BackgroundProcessGuard {
            session: self,
            background_process,
        };

        let command_output = guard
            .session
            .run_cmd_and_collect_output(&guard.background_process.start)?;
        if command_output.exit_status != 0 {
            return Err(Error::NonZeroExit {
                exit_status: command_output.exit_status,
                output: truncate_output_for_error(&command_output.output),
            });
        }

        guard.session.wait_until_check_matches(
            &guard.background_process.ready,
            READY_TIMEOUT,
            "background process to become ready",
        )?;

        test_ops(guard.session)
    }

    pub(super) fn shutdown(&mut self) -> Result<(), Error> {
        self.pty_session
            .send_line(&self.desc.exit_cmd)
            .map_err(Error::from)?;

        self.pty_session.process.wait().map_err(Error::from)?;

        Ok(())
    }

    fn output_error(error: &Error) {
        match error {
            Error::Pty(rexpect::error::Error::EOF {
                expected,
                got,
                exit_code,
            }) => {
                println!("=== EOF Error Details ===");
                println!("Expected: {}", expected);
                println!("Got: {}", got);
                println!("Exit code: {:?}", exit_code);
                println!("========================");
            }
            Error::Timeout {
                expected,
                got,
                timeout,
            } => {
                println!("=== Timeout Error Details ===");
                println!("Expected: {}", expected);
                println!("Got: {}", got);
                println!("Timeout: {:?}", timeout);
                println!("============================");
            }
            Error::UnexpectedOutput { expected, got } => {
                println!("=== Unexpected Output ===");
                println!("Expected: {}", expected);
                println!("Output before prompt:\n{}", got);
                println!("=========================");
            }
            Error::Protocol { reason, got } => {
                println!("=== Protocol Error Details ===");
                println!("Reason: {}", reason);
                println!("Got: {}", got);
                println!("==============================");
            }
            Error::NonZeroExit {
                exit_status,
                output,
            } => {
                println!("=== Command Exit Error Details ===");
                println!("Exit status: {}", exit_status);
                println!("Output:\n{}", output);
                println!("==================================");
            }
            Error::Pty(reason) => {
                println!("=== PTY Error Details ===");
                println!("Reason: {}", reason);
                println!("=========================");
            }
            Error::Aggregated {
                summary,
                collections,
            } => {
                println!("=== Aggregated Error Details ===");
                println!("Summary: {}", summary);
                for (name, error) in collections {
                    println!("--- {}", name);
                    Self::output_error(error);
                }
                println!("================================");
            }
        }
    }

    fn run_cmd_and_collect_output(&mut self, command: &str) -> Result<CommandOutput, Error> {
        let exit_marker = self.uuid.as_str();
        let quoted_command = format!("'{}'", command.replace('\'', r#"'"'"'"#));
        let wrapped_command = format!(
            r#"eval -- {}; __exit_code=$?; printf '\n{}%s\n' "$__exit_code""#,
            quoted_command, exit_marker
        );

        println!("--> Running: {}", command);
        self.pty_session
            .send_line(&wrapped_command)
            .map_err(Error::from)?;
        self.pty_session
            .exp_string(&wrapped_command)
            .map_err(Error::from)?;

        let unread = match self
            .pty_session
            .exp_string(&self.desc.prompt)
            .map_err(Error::from)
        {
            Ok(unread) => unread,
            Err(error) => {
                Self::output_error(&error);
                return Err(error);
            }
        };

        let (command_output, exit_status) = Self::parse_command_output(&unread, exit_marker)?;

        Ok(CommandOutput {
            exit_status,
            output: command_output,
        })
    }

    fn parse_command_output(output: &str, exit_marker: &str) -> Result<(String, i32), Error> {
        let sanitize_output = |str| truncate_output_for_error(&clean_output(str));

        let Some((command_output, exit_status_text)) = output.rsplit_once(exit_marker) else {
            return Err(Error::Protocol {
                reason: "missing exit status marker".to_string(),
                got: sanitize_output(output),
            });
        };

        let Some(exit_status_line) = exit_status_text.lines().next() else {
            return Err(Error::Protocol {
                reason: "missing numeric exit status".to_string(),
                got: sanitize_output(command_output),
            });
        };
        let Ok(exit_status) = exit_status_line.parse::<i32>() else {
            return Err(Error::Protocol {
                reason: format!("invalid numeric exit status: {}", exit_status_line),
                got: sanitize_output(command_output),
            });
        };

        Ok((clean_output(command_output).trim().to_string(), exit_status))
    }

    fn stop_background_process(
        &mut self,
        background_process: &BackgroundProcess,
    ) -> Result<(), Error> {
        let command_output = self.run_cmd_and_collect_output(&background_process.stop)?;
        if command_output.exit_status != 0 {
            return Err(Error::NonZeroExit {
                exit_status: command_output.exit_status,
                output: truncate_output_for_error(&command_output.output),
            });
        }

        self.wait_until_check_matches(
            &background_process.stopped,
            STOP_TIMEOUT,
            "background process to stop",
        )
    }

    fn wait_until_check_matches(
        &mut self,
        command_check: &CommandCheck,
        timeout: Duration,
        expected_state: &str,
    ) -> Result<(), Error> {
        let start_time = Instant::now();

        loop {
            let command_output = self.run_cmd_and_collect_output(&command_check.cmd)?;

            if command_output.exit_status == 0
                && command_output
                    .output
                    .contains(command_check.expect.as_ref())
            {
                return Ok(());
            }

            if start_time.elapsed() >= timeout {
                return Err(Error::Timeout {
                    expected: expected_state.to_string(),
                    got: format!(
                        "exit status: {}\nexpected output: {}\nactual output:\n{}",
                        command_output.exit_status,
                        command_check.expect,
                        truncate_output_for_error(&command_output.output)
                    ),
                    timeout,
                });
            }

            std::thread::sleep(POLL_INTERVAL);
        }
    }
}

/// Describes how to manage a background process during a test.
pub struct BackgroundProcess {
    /// Starts the background process.
    start: Cow<'static, str>,
    /// Checks whether the background process is ready for test operations.
    ready: CommandCheck,
    /// Stops the background process.
    stop: Cow<'static, str>,
    /// Checks whether the background process has fully stopped.
    stopped: CommandCheck,
}

impl BackgroundProcess {
    /// Creates a new background process descriptor.
    pub fn new(
        start: impl Into<Cow<'static, str>>,
        ready: CommandCheck,
        stop: impl Into<Cow<'static, str>>,
        stopped: CommandCheck,
    ) -> Self {
        Self {
            start: start.into(),
            ready,
            stop: stop.into(),
            stopped,
        }
    }
}

/// Describes a command-based state check for a background process.
pub struct CommandCheck {
    /// Runs to observe whether the background process reached the target state.
    cmd: Cow<'static, str>,
    /// Must appear in the command output for the check to succeed.
    expect: Cow<'static, str>,
}

impl CommandCheck {
    /// Creates a new command-based state check.
    pub fn new(cmd: impl Into<Cow<'static, str>>, expect: impl Into<Cow<'static, str>>) -> Self {
        Self {
            cmd: cmd.into(),
            expect: expect.into(),
        }
    }
}

struct CommandOutput {
    exit_status: i32,
    output: String,
}

struct BackgroundProcessGuard<'s> {
    session: &'s mut Session,
    background_process: BackgroundProcess,
}

impl Drop for BackgroundProcessGuard<'_> {
    fn drop(&mut self) {
        let _ = self
            .session
            .stop_background_process(&self.background_process);
    }
}

const READY_TIMEOUT: Duration = Duration::from_secs(30);
const STOP_TIMEOUT: Duration = Duration::from_secs(10);
const POLL_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(test)]
mod tests {
    use super::{Error, Session};

    #[test]
    fn parse_command_output_extracts_status_from_marker() {
        let exit_marker = "00000000-0000-0000-0000-000000000000";
        let command_output = format!("hello\n{}17\n", exit_marker);

        let (output, exit_status) =
            Session::parse_command_output(&command_output, exit_marker).unwrap();

        assert_eq!(output, "hello");
        assert_eq!(exit_status, 17);
    }

    #[test]
    fn parse_command_output_rejects_missing_marker() {
        let exit_marker = "00000000-0000-0000-0000-000000000000";
        let error = Session::parse_command_output("hello", exit_marker).unwrap_err();

        match error {
            Error::Protocol { reason, got } => {
                assert_eq!(reason, "missing exit status marker");
                assert_eq!(got, "hello");
            }
            other => panic!("unexpected error: {:?}", other),
        }
    }
}
