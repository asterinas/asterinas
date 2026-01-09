// SPDX-License-Identifier: MPL-2.0

use rexpect::session::PtySession;

use super::Error;

/// Describes a runtime session configuration.
///
/// This struct uses the builder pattern to configure session parameters including
/// the prompt string, commands to enter/exit the session, etc.
///
/// # Example
///
/// The session descriptor uses a fluent builder API:
///
/// ```rust
/// use nixos_test_framework::SessionDesc;
///
/// let desc = SessionDesc::new()
///     .expect_prompt("/ #")
///     .cmd_to_enter("podman run -it alpine")
///     .cmd_to_exit("exit");
/// ```
///
/// # Fields
///
/// - **prompt**: The string that indicates the session is ready for commands
/// - **enter_command**: The command used to enter this session context
/// - **exit_cmd**: The command used to leave the session
pub struct SessionDesc {
    prompt: &'static str,
    enter_command: &'static str,
    exit_cmd: &'static str,
}

impl SessionDesc {
    /// Creates a new session descriptor with empty fields.
    ///
    /// Use the builder methods to configure the session before use.
    pub fn new() -> Self {
        Self {
            prompt: "",
            enter_command: "",
            exit_cmd: "",
        }
    }

    /// Sets the prompt string that indicates the session is ready for commands.
    ///
    /// The prompt is used by the framework to detect when command execution has completed.
    pub fn expect_prompt(mut self, prompt: &'static str) -> Self {
        self.prompt = prompt;
        self
    }

    /// Sets the command used to enter this session.
    ///
    /// This command will be executed in the parent session to transition into
    /// the new session context.
    pub fn cmd_to_enter(mut self, enter_command: &'static str) -> Self {
        self.enter_command = enter_command;
        self
    }

    /// Sets the command used to exit this session.
    ///
    /// This command will be executed to return to the parent session.
    pub fn cmd_to_exit(mut self, exit_cmd: &'static str) -> Self {
        self.exit_cmd = exit_cmd;
        self
    }
}

impl Default for SessionDesc {
    fn default() -> Self {
        Self::new()
    }
}

/// A session for interacting with QEMU virtual machine environments.
///
/// `Session` provides powerful APIs for writing interactive tests.
/// It represents the *current* interactive context within the QEMU virtual
/// machine. Unlike a raw PTY session, it is stateful and understands the concept
/// of nested environments.
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

    fn output_error(error: &Error) {
        match error {
            Error::EOF {
                expected,
                got,
                exit_code,
            } => {
                eprintln!("=== EOF Error Details ===");
                eprintln!("Expected: {}", expected);
                eprintln!(
                    "Got: {}",
                    String::from_utf8_lossy(&strip_ansi_escapes::strip(got))
                );
                eprintln!("Exit code: {:?}", exit_code);
                eprintln!("========================");
            }
            Error::Timeout {
                expected,
                got,
                timeout,
            } => {
                eprintln!("=== Timeout Error Details ===");
                eprintln!("Expected: {}", expected);
                eprintln!(
                    "Got: {}",
                    String::from_utf8_lossy(&strip_ansi_escapes::strip(got))
                );
                eprintln!("Timeout: {:?}", timeout);
                eprintln!("============================");
            }
            _ => {}
        }
    }

    /// Executes a command and waits for the session's prompt to return.
    ///
    /// This is the most basic command execution method. It sends the command,
    /// waits for the prompt to appear again.
    ///
    /// If a timeout occurred or the session ended unexpectedly, returns an error.
    ///
    /// # Example
    ///
    /// ```rust
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
        self.pty_session.send_line(command)?;
        // Read and consume the echoed command line
        self.pty_session.exp_string(command).unwrap();

        if let Err(e) = self.pty_session.exp_string(self.desc.prompt) {
            Self::output_error(&e);
            return Err(e);
        }

        Ok(())
    }

    /// Executes a command and verifies that specific text appears in the output.
    ///
    /// This method is useful for validating that commands produce expected output.
    /// It executes the command and checks that the `expected` string appears
    /// anywhere in the output before the prompt returns.
    ///
    /// If the expected string was not found, or a timeout/EOF occurred, returns an error.
    ///
    /// # Example
    ///
    /// ```rust
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
        self.pty_session.send_line(command)?;
        // Read and consume the echoed command line
        self.pty_session.exp_string(command).unwrap();

        match self.pty_session.exp_string(self.desc.prompt) {
            Ok(unread) => {
                let cleaned_unread =
                    String::from_utf8_lossy(&strip_ansi_escapes::strip(&unread)).to_string();
                if !cleaned_unread.contains(expected) {
                    eprintln!("=== Unexpected Output ===");
                    eprintln!("Expected: {}", expected);
                    eprintln!("Output before prompt:\n{}", cleaned_unread);
                    eprintln!("=========================");
                    return Err(Error::EOF {
                        expected: expected.to_string(),
                        got: cleaned_unread,
                        exit_code: None,
                    });
                }
            }
            Err(e) => {
                Self::output_error(&e);
                return Err(e);
            }
        }

        Ok(())
    }

    pub(super) fn run<F>(&mut self, test_ops: F) -> Result<(), Error>
    where
        F: FnOnce(&mut Session) -> Result<(), Error>,
    {
        (test_ops)(self)
    }

    /// Enters a nested session, runs operations, and automatically exits.
    ///
    /// This method is used to work with nested environments like containers, SSH sessions,
    /// or any interactive shell. It handles entering the session, running your test operations,
    /// and properly exiting back to the parent session - even if errors occur.
    ///
    /// If entering the session, running operations, or exiting fails, returns an error.
    ///
    /// # Example
    ///
    /// ```rust
    /// use nixos_test_framework::*;
    ///
    /// fn container_test(nixos_shell: &mut Session) -> Result<(), Error> {
    ///     // Define the container session
    ///     let container_session_desc = SessionDesc::new()
    ///         .expect_prompt("/ #")
    ///         .cmd_to_enter("podman run -it docker.io/library/alpine")
    ///         .cmd_to_exit("exit");
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

        if let Err(e) = self.run_cmd(self.desc.enter_command) {
            self.desc = old_desc;
            return Err(e);
        }

        let res = self.run(test_ops);

        let exit_cmd = self.desc.exit_cmd;
        self.desc = old_desc;
        let exit_res = self.run_cmd(exit_cmd);

        res?;
        exit_res?;

        Ok(())
    }

    pub(super) fn shutdown(&mut self) -> Result<(), Error> {
        self.pty_session.send_line(self.desc.exit_cmd)?;

        self.pty_session.process.wait()?;

        Ok(())
    }
}
