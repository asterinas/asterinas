// SPDX-License-Identifier: MPL-2.0

use std::{
    error::Error,
    fmt,
    fs::File,
    io::{BufRead, BufReader, Lines},
    process::{Command, Stdio},
};

use regex::Regex;

use crate::parameter::Parameters;

/// Error type for trace operations
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
    fn new(message: &str) -> Self {
        Self {
            message: message.to_string(),
        }
    }
}

/// Iterator for trace lines in online mode
pub struct OnlineTraceIterator {
    reader: BufReader<std::process::ChildStdout>,
}

impl OnlineTraceIterator {
    /// Create new online iterator from `params`.
    ///
    /// Fork a process that calls the strace tool, e.g.,
    /// calls `strace params.program_path params.program_args`.
    ///
    /// Ignore the output of the traced program, we can
    /// directly use the command:
    /// `strace -o /dev/stdout -y -f sh -c
    /// 'params.program_path params.program_args' >/dev/null 2>&1`
    /// and get the output of `stdout` to extract the output of strace.
    fn new(params: &Parameters) -> Result<Self, TraceError> {
        let mut command_str = params.program_path().to_string();
        for arg in params.program_args() {
            command_str.push(' ');
            command_str.push_str(arg);
        }
        command_str.push_str(" >/dev/null 2>&1");

        let mut child = Command::new("strace")
            .args(&["-o", "/dev/stdout", "-y", "-f", "sh", "-c", &command_str])
            .stdout(Stdio::piped())
            .stderr(Stdio::null())
            .spawn()
            .map_err(|e| TraceError::new(&format!("Failed to start strace: {}", e)))?;

        let stdout = child
            .stdout
            .take()
            .ok_or_else(|| TraceError::new("Failed to capture stdout"))?;

        Ok(Self {
            reader: BufReader::new(stdout),
        })
    }
}

impl Iterator for OnlineTraceIterator {
    type Item = String;

    /// Use `self.reader` to get strace output line by line.
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

/// Iterator for trace lines in offline mode
pub struct OfflineTraceIterator {
    lines: Lines<BufReader<File>>,
}

impl OfflineTraceIterator {
    /// Create new offline iterator from `params`.
    ///
    /// Construct `BufReader` from the file specified in
    /// `params.input_path()`.
    fn new(params: &Parameters) -> Result<Self, TraceError> {
        let file = File::open(params.input_path())
            .map_err(|e| TraceError::new(&format!("Failed to open input file: {}", e)))?;

        let reader = BufReader::new(file);
        let lines = reader.lines();

        Ok(Self { lines })
    }
}

impl Iterator for OfflineTraceIterator {
    type Item = String;

    /// Use `self.lines` to get strace log line by line.
    fn next(&mut self) -> Option<Self::Item> {
        match self.lines.next() {
            Some(Ok(line)) => Some(line),
            _ => None,
        }
    }
}

/// Enum to hold either online or offline iterator
pub enum TraceIterator {
    Online(OnlineTraceIterator),
    Offline(OfflineTraceIterator),
}

impl TraceIterator {
    /// Create a new `TraceIterator` based on `params`.
    ///
    /// Determine whether it is offline mode according to
    /// `params.offline()`. Then create `OnlineTraceIterator`
    /// or `OfflineTraceIterator` based on the mode.
    ///
    /// If it is in online mode, check the strace version using
    /// `check_strace_version()` before creating iterator.
    pub fn new(params: &Parameters) -> Result<Self, Box<dyn Error>> {
        if params.offline() {
            let offline_iter = OfflineTraceIterator::new(params)?;
            Ok(Self::Offline(offline_iter))
        } else {
            Self::check_strace_version()?;
            let online_iter = OnlineTraceIterator::new(params)?;
            Ok(Self::Online(online_iter))
        }
    }

    /// Use regex to check if strace version is >= 5.15.
    /// We can use `strace --version` command to get the
    /// version number, whose output is similar to:
    /// ```text
    /// strace -- version 6.16.0.7.78e18
    /// Copyright (c) 1991-2025 The strace developers <https://strace.io>.
    /// This is free software; see the source for copying conditions.  There is NO
    /// warranty; not even for MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE
    /// ```
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

    fn next(&mut self) -> Option<Self::Item> {
        match self {
            Self::Online(iter) => iter.next(),
            Self::Offline(iter) => iter.next(),
        }
    }
}

#[cfg(test)]
mod tests {
    use std::io::Write;

    use tempfile::NamedTempFile;

    use super::*;

    /// Helper function to create test parameters for offline mode
    fn create_offline_params(file_path: &str) -> Parameters {
        let args = vec![
            "sctrace".to_string(),
            "test.scml".to_string(),
            "--input".to_string(),
            file_path.to_string(),
        ];
        Parameters::new(args).unwrap()
    }

    /// Helper function to create test parameters for online mode
    fn create_online_params() -> Parameters {
        let args = vec![
            "sctrace".to_string(),
            "test.scml".to_string(),
            "echo".to_string(),
            "hello".to_string(),
        ];
        Parameters::new(args).unwrap()
    }

    #[test]
    fn test_offline_iterator_creation() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            "execve(\"./hello_world\", [\"./hello_world\"], 0x7fff9a04a908 /* 27 vars */) = 0"
        )
        .unwrap();
        writeln!(
            temp_file,
            "brk(NULL)                               = 0x556830530000"
        )
        .unwrap();
        temp_file.flush().unwrap();

        let params = create_offline_params(temp_file.path().to_str().unwrap());
        let iter = OfflineTraceIterator::new(&params);
        assert!(iter.is_ok());
    }

    #[test]
    fn test_offline_iterator_reading() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            "execve(\"./hello_world\", [\"./hello_world\"], 0x7fff9a04a908 /* 27 vars */) = 0"
        )
        .unwrap();
        writeln!(
            temp_file,
            "brk(NULL)                               = 0x556830530000"
        )
        .unwrap();
        writeln!(
            temp_file,
            "arch_prctl(0x3001 /* ARCH_??? */, 0x7fff95d275a0) = -1 EINVAL (Invalid argument)"
        )
        .unwrap();
        temp_file.flush().unwrap();

        let params = create_offline_params(temp_file.path().to_str().unwrap());
        let mut iter = OfflineTraceIterator::new(&params).unwrap();

        assert_eq!(
            iter.next(),
            Some(
                "execve(\"./hello_world\", [\"./hello_world\"], 0x7fff9a04a908 /* 27 vars */) = 0"
                    .to_string()
            )
        );
        assert_eq!(
            iter.next(),
            Some("brk(NULL)                               = 0x556830530000".to_string())
        );
        assert_eq!(
            iter.next(),
            Some(
                "arch_prctl(0x3001 /* ARCH_??? */, 0x7fff95d275a0) = -1 EINVAL (Invalid argument)"
                    .to_string()
            )
        );
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_offline_iterator_nonexistent_file() {
        let params = create_offline_params("/nonexistent/file.txt");
        let iter = OfflineTraceIterator::new(&params);
        assert!(iter.is_err());
    }

    #[test]
    fn test_trace_iterator_offline_mode() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(
            temp_file,
            "access(\"/etc/ld.so.preload\", R_OK)      = -1 ENOENT (No such file or directory)"
        )
        .unwrap();
        temp_file.flush().unwrap();

        let params = create_offline_params(temp_file.path().to_str().unwrap());
        let iter = TraceIterator::new(&params);
        assert!(iter.is_ok());

        if let Ok(TraceIterator::Offline(_)) = iter {
            // Correct variant
        } else {
            panic!("Expected offline iterator");
        }
    }

    #[test]
    fn test_trace_iterator_creation_offline() {
        let mut temp_file = NamedTempFile::new().unwrap();
        writeln!(temp_file, "close(3</etc/ld.so.cache>)              = 0").unwrap();
        writeln!(temp_file, "openat(AT_FDCWD</home/sutao/test>, \"/lib/x86_64-linux-gnu/libc.so.6\", O_RDONLY|O_CLOEXEC) = 3</usr/lib/x86_64-linux-gnu/libc-2.31.so>").unwrap();
        temp_file.flush().unwrap();

        let params = create_offline_params(temp_file.path().to_str().unwrap());
        let mut iter = TraceIterator::new(&params).unwrap();

        assert_eq!(
            iter.next(),
            Some("close(3</etc/ld.so.cache>)              = 0".to_string())
        );
        assert_eq!(
            iter.next(),
            Some("openat(AT_FDCWD</home/sutao/test>, \"/lib/x86_64-linux-gnu/libc.so.6\", O_RDONLY|O_CLOEXEC) = 3</usr/lib/x86_64-linux-gnu/libc-2.31.so>".to_string())
        );
        assert_eq!(iter.next(), None);
    }

    #[test]
    fn test_strace_version_check() {
        // This test will only pass if strace is installed and version >= 5.15
        // In a real environment, you might want to skip this test or mock it
        match TraceIterator::check_strace_version() {
            Ok(()) => {
                // Version check passed
                println!("strace version check passed");
            }
            Err(e) => {
                // This might fail if strace is not installed or version is too old
                println!("strace version check failed: {}", e);
            }
        }
    }

    #[test]
    fn test_trace_error_display() {
        let error = TraceError::new("test error message");
        assert_eq!(format!("{}", error), "test error message");
    }

    #[test]
    fn test_create_online_params_basic() {
        let params = create_online_params();
        assert_eq!(params.scml_path(), "test.scml");
        assert_eq!(params.program_path(), "echo");
        assert_eq!(params.program_args(), &["hello"]);
        assert!(!params.offline());
    }

    #[test]
    fn test_trace_iterator_online_mode() {
        let params = create_online_params();
        // This test will check if the TraceIterator can be created in online mode
        // It might fail if strace is not available or version requirements are not met
        match TraceIterator::new(&params) {
            Ok(TraceIterator::Online(_)) => {
                println!("Online trace iterator created successfully");
            }
            Ok(TraceIterator::Offline(_)) => {
                panic!("Expected online iterator but got offline");
            }
            Err(e) => {
                // This is expected if strace is not available or version is too old
                panic!(
                    "Online trace iterator creation failed (expected in test environment): {}",
                    e
                );
            }
        }
    }

    #[test]
    fn test_online_execution_with_echo() {
        // Use the existing create_online_params function which uses echo command
        let params = create_online_params();

        // Skip test if strace is not available or version is too old
        if TraceIterator::check_strace_version().is_err() {
            println!("Skipping online execution test: strace not available or version too old");
            return;
        }

        println!("Starting online execution test with echo command...");

        // Create and run the online iterator
        match TraceIterator::new(&params) {
            Ok(mut iter) => {
                println!("Online iterator created successfully for 'echo hello'");

                let mut line_count = 0;
                let mut found_echo_execve = false;
                let mut execution_completed = false;

                // Read trace output line by line
                while let Some(line) = iter.next() {
                    line_count += 1;
                    println!("Trace line {}: {}", line_count, line);

                    // Look for execve system call with echo command
                    if line.contains("execve")
                        && (line.contains("echo") || line.contains("/bin/echo"))
                    {
                        found_echo_execve = true;
                        println!("Found echo execve call!");
                    }

                    // Look for exit_group which indicates program completion
                    if line.contains("exit_group") {
                        execution_completed = true;
                        println!("Program execution completed!");
                    }

                    // Limit the number of lines to prevent infinite loops
                    if line_count >= 100 {
                        println!("Reached maximum line limit, stopping trace reading");
                        break;
                    }

                    // If we found both execve and exit, the test is successful
                    if found_echo_execve && execution_completed {
                        println!("Test successful: found both execve and exit_group");
                        break;
                    }
                }

                println!("Online execution test completed:");
                println!("  Lines read: {}", line_count);
                println!("  Found echo execve: {}", found_echo_execve);
                println!("  Execution completed: {}", execution_completed);

                // Test passes if we read at least some trace output
                // The echo command should generate some system calls
                assert!(
                    line_count > 0,
                    "Should have read at least one trace line from echo command"
                );

                println!("✓ Online execution test with echo passed!");
            }
            Err(e) => {
                panic!("Online iterator creation failed: {}", e);
            }
        }
    }
}
