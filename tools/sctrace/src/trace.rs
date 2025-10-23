// SPDX-License-Identifier: MPL-2.0

//! Trace module for system call tracing using strace.
//!
//! This module provides functionality to trace system calls either online (by spawning
//! a traced process) or offline (by reading from a pre-existing strace log file).

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{BufRead, BufReader, Lines},
    os::unix::io::FromRawFd,
    process::Command,
};

use regex::Regex;

/// Error type for trace operations.
///
/// This error type is returned when operations related to tracing fail,
/// such as starting strace, reading files, or parsing version information.
#[derive(Debug)]
pub struct TraceError {
    message: String,
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for TraceError {}

impl TraceError {
    /// Creates a new `TraceError` with the given message.
    ///
    /// # Arguments
    ///
    /// * `message` - A string slice describing the error
    fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

/// Iterator for trace lines in online mode.
///
/// This iterator spawns a new process and traces it using strace,
/// yielding each line of strace output as it becomes available.
pub struct OnlineTraceIterator {
    reader: BufReader<File>,
}

impl OnlineTraceIterator {
    /// Creates a new online iterator from the given parameters.
    ///
    /// This method spawns a process that invokes strace to trace the specified
    /// program with the given arguments. The strace output is captured via a pipe
    /// using the `/proc/self/fd/N` mechanism.
    ///
    /// # Arguments
    ///
    /// * `program_path` - Path to the program to trace
    /// * `program_args` - Arguments to pass to the traced program
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` on success, or a `TraceError` if:
    /// - Pipe creation fails
    /// - strace fails to start
    ///
    /// # Example Command
    ///
    /// The underlying command executed is:
    /// ```text
    /// strace -o /proc/self/fd/3 -yy -f sh -c 'program_path args...'
    /// ```
    fn new(program_path: &str, program_args: Vec<&str>) -> Result<Self, TraceError> {
        let mut command_str = program_path.to_string();
        for arg in program_args {
            command_str.push(' ');
            command_str.push_str(arg);
        }

        // Create pipe
        let (read_fd, write_fd) = nix::unistd::pipe()
            .map_err(|e| TraceError::new(&format!("Failed to create pipe: {}", e)))?;

        // Convert read end to File
        let read_file = unsafe { std::fs::File::from_raw_fd(read_fd) };

        // Start strace, using /proc/self/fd/N to access the write end
        Command::new("strace")
            .args(&[
                "-o",
                &format!("/proc/self/fd/{}", write_fd),
                "-yy",
                "-f",
                "sh",
                "-c",
                &command_str,
            ])
            .spawn()
            .map_err(|e| TraceError::new(&format!("Failed to start strace: {}", e)))?;

        // Close write end (strace has already inherited it)
        nix::unistd::close(write_fd).ok();

        Ok(Self {
            reader: BufReader::new(read_file),
        })
    }
}

impl Iterator for OnlineTraceIterator {
    type Item = String;

    /// Returns the next line of strace output.
    ///
    /// Reads lines from the strace output, removing trailing newlines.
    /// Returns `None` when the traced process terminates or an error occurs.
    fn next(&mut self) -> Option<Self::Item> {
        let mut line = String::new();
        match self.reader.read_line(&mut line) {
            Ok(0) => None, // EOF
            Ok(_) => {
                // Remove trailing newline
                if line.ends_with('\n') {
                    line.pop();
                    if line.ends_with('\r') {
                        line.pop();
                    }
                }
                Some(line)
            }
            Err(_) => None,
        }
    }
}

/// Iterator for trace lines in offline mode.
///
/// This iterator reads strace output from a pre-existing log file,
/// yielding each line sequentially.
pub struct OfflineTraceIterator {
    lines: Lines<BufReader<File>>,
}

impl OfflineTraceIterator {
    /// Creates a new offline iterator from the given file path.
    ///
    /// Opens and reads the strace log file specified by `input_path`.
    ///
    /// # Arguments
    ///
    /// * `input_path` - Path to the strace log file
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` on success, or a `TraceError` if the file cannot be opened.
    fn new(input_path: &str) -> Result<Self, TraceError> {
        let file = File::open(input_path)
            .map_err(|e| TraceError::new(&format!("Failed to open input file: {}", e)))?;

        let reader = BufReader::new(file);
        let lines = reader.lines();

        Ok(Self { lines })
    }
}

impl Iterator for OfflineTraceIterator {
    type Item = String;

    /// Returns the next line from the strace log file.
    ///
    /// Returns `None` when the end of the file is reached or an error occurs.
    fn next(&mut self) -> Option<Self::Item> {
        match self.lines.next() {
            Some(Ok(line)) => Some(line),
            _ => None,
        }
    }
}

/// Enum to hold either online or offline trace iterator.
///
/// This enum provides a unified interface for both online (live tracing)
/// and offline (log file reading) modes of operation.
pub enum TraceIterator {
    /// Online mode: trace a running process
    Online(OnlineTraceIterator),
    /// Offline mode: read from a log file
    Offline(OfflineTraceIterator),
}

impl TraceIterator {
    /// Creates a new online trace iterator.
    ///
    /// This method verifies that strace version is >= 5.15, then spawns
    /// a traced process using strace.
    ///
    /// # Arguments
    ///
    /// * `program_path` - Path to the program to trace
    /// * `program_args` - Arguments to pass to the traced program
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` on success, or an error if:
    /// - strace version check fails
    /// - the traced process cannot be started
    ///
    /// # Errors
    ///
    /// Returns an error if strace version is < 5.15 or if process spawning fails.
    pub fn new_online(program_path: &str, program_args: Vec<&str>) -> Result<Self, Box<dyn Error>> {
        Self::check_strace_version()?;
        let online_iter = OnlineTraceIterator::new(program_path, program_args)?;
        Ok(Self::Online(online_iter))
    }

    /// Creates a new offline trace iterator.
    ///
    /// Opens the specified strace log file for reading.
    ///
    /// # Arguments
    ///
    /// * `input_path` - Path to the strace log file
    ///
    /// # Returns
    ///
    /// Returns `Ok(Self)` on success, or an error if the file cannot be opened.
    ///
    /// # Errors
    ///
    /// Returns an error if the file does not exist or cannot be opened.
    pub fn new_offline(input_path: &str) -> Result<Self, Box<dyn Error>> {
        let offline_iter = OfflineTraceIterator::new(input_path)?;
        Ok(Self::Offline(offline_iter))
    }

    /// Checks if strace version is >= 5.15.
    ///
    /// Executes `strace --version` and parses the output to verify the version
    /// meets the minimum requirement of 5.15.
    ///
    /// # Returns
    ///
    /// Returns `Ok(())` if strace version is >= 5.15.
    ///
    /// # Errors
    ///
    /// Returns a `TraceError` if:
    /// - strace command fails to execute
    /// - version output cannot be parsed
    /// - version is < 5.15
    fn check_strace_version() -> Result<(), TraceError> {
        let output = Command::new("strace")
            .arg("--version")
            .output()
            .map_err(|e| TraceError::new(&format!("Failed to run strace --version: {}", e)))?;

        let version_output = String::from_utf8_lossy(&output.stdout);

        // Use regex to extract version number
        let re = Regex::new(r"strace.*version\s+(\d+)\.(\d+)")
            .map_err(|e| TraceError::new(&format!("Failed to compile regex: {}", e)))?;

        if let Some(caps) = re.captures(&version_output) {
            let major: u32 = caps[1]
                .parse()
                .map_err(|_| TraceError::new("Failed to parse major version"))?;
            let minor: u32 = caps[2]
                .parse()
                .map_err(|_| TraceError::new("Failed to parse minor version"))?;

            if major > 5 || (major == 5 && minor >= 15) {
                Ok(())
            } else {
                Err(TraceError::new(&format!(
                    "strace version {}.{} is too old, requires >= 5.15",
                    major, minor
                )))
            }
        } else {
            Err(TraceError::new("Failed to parse strace version output"))
        }
    }
}

impl Iterator for TraceIterator {
    type Item = String;

    /// Returns the next trace line from either online or offline source.
    ///
    /// Delegates to the appropriate iterator based on the variant.
    ///
    /// # Returns
    ///
    /// Returns `Some(String)` containing the next trace line, or `None` when
    /// the iterator is exhausted.
    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Online(iter) => iter.next(),
            Self::Offline(iter) => iter.next(),
        }
    }
}
