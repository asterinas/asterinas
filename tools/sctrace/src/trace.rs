// SPDX-License-Identifier: MPL-2.0

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{BufRead, BufReader, Lines, Read},
    os::unix::io::FromRawFd,
    path::Path,
    process::Command,
};

#[derive(Debug)]
pub struct TraceError {
    message: String,
}

impl fmt::Display for TraceError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<TraceError> for String {
    fn from(err: TraceError) -> Self {
        err.message
    }
}

impl Error for TraceError {}

impl TraceError {
    fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

/// A stream of strace log entries.
pub struct StraceLogStream(Box<dyn BufRead>);

impl Read for StraceLogStream {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        self.0.read(buf)
    }
}

impl BufRead for StraceLogStream {
    fn fill_buf(&mut self) -> std::io::Result<&[u8]> {
        self.0.fill_buf()
    }

    fn consume(&mut self, amt: usize) {
        self.0.consume(amt)
    }
}

impl StraceLogStream {
    /// Creates a new stream by opening an existing strace log file.
    pub fn open_file<P: AsRef<Path>>(path: P) -> Result<Self, TraceError> {
        let file = File::open(path.as_ref())
            .map_err(|e| TraceError::new(&format!("Failed to open log file: {}", e)))?;
        Ok(Self(Box::new(BufReader::new(file))))
    }

    /// Creates a new stream by running a new command with strace.
    pub fn run_cmd<P: AsRef<Path>>(path: P, args: Vec<&str>) -> Result<Self, TraceError> {
        let mut command_str = path.as_ref().to_string_lossy().to_string();
        for arg in args {
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
            .args([
                "-o",
                &format!("/proc/self/fd/{}", write_fd),
                "-yy",
                "-f",
                "sh",
                "-c",
                &command_str,
            ])
            .spawn()
            .map_err(|e| {
                // Clean up file descriptors
                nix::unistd::close(read_fd).ok();
                nix::unistd::close(write_fd).ok();

                if e.kind() == std::io::ErrorKind::NotFound {
                    TraceError::new(
                        "strace command not found. Please install strace:\n\
                        - Debian/Ubuntu: sudo apt-get install strace\n\
                        - Fedora/RHEL: sudo dnf install strace",
                    )
                } else {
                    TraceError::new(&format!(
                        "Failed to start strace: {}\n\
                        If this is a permission error, try:\n\
                        sudo sctrace <pattern_file> -- {}",
                        e, command_str
                    ))
                }
            })?;

        // Close write end (strace has already inherited it)
        nix::unistd::close(write_fd).ok();

        Ok(Self(Box::new(BufReader::new(read_file))))
    }

    /// Creates a new stream by a string of strace log.
    pub fn from_string(log_str: &str) -> Result<Self, TraceError> {
        let cursor = std::io::Cursor::new(log_str.to_string().into_bytes());
        Ok(Self(Box::new(BufReader::new(cursor))))
    }

    /// Returns an iterator over the lines of this stream.
    pub fn lines(self) -> Lines<Self> {
        BufRead::lines(self)
    }
}
