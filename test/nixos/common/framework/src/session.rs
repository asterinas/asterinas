// SPDX-License-Identifier: MPL-2.0

use std::borrow::Cow;

use rexpect::session::PtySession;

use super::Error;

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
    desc: SessionDesc,
    pty_session: PtySession,
}

impl Session {
    /// Creates a new session with the given PTY session and descriptor.
    pub(super) fn new(desc: SessionDesc, pty_session: PtySession) -> Self {
        Self { desc, pty_session }
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
                let cleaned_unread = Self::clean_output(&unread);
                if !cleaned_unread.contains(expected) {
                    let error = Error::ExpectMismatch {
                        expected: expected.to_string(),
                        got: cleaned_unread,
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
                println!("Got: {}", Self::clean_output(got));
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
                println!("Got: {}", Self::clean_output(got));
                println!("Timeout: {:?}", timeout);
                println!("============================");
            }
            Error::ExpectMismatch { expected, got } => {
                println!("=== Unexpected Output ===");
                println!("Expected: {}", expected);
                println!("Output before prompt:\n{}", got);
                println!("=========================");
            }
            Error::Protocol { reason, got } => {
                println!("=== Protocol Error Details ===");
                println!("Reason: {}", reason);
                println!("Got: {}", Self::clean_output(got));
                println!("==============================");
            }
            Error::NonZeroExit {
                exit_status,
                output,
            } => {
                println!("=== Command Exit Error Details ===");
                println!("Exit status: {}", exit_status);
                println!("Output:\n{}", Self::clean_output(output));
                println!("==================================");
            }
            Error::Pty(reason) => {
                println!("=== PTY Error Details ===");
                println!("Reason: {}", reason);
                println!("=========================");
            }
        }
    }

    fn clean_output(output: &str) -> String {
        String::from_utf8_lossy(&strip_ansi_escapes::strip(output)).to_string()
    }
}
