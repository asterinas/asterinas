// SPDX-License-Identifier: MPL-2.0

//! Syscall-Compliance-Trace(sctrace) for tracing and validating system calls against SCML patterns.
//!
//! This binary provides a comprehensive solution for analyzing system call behavior by comparing
//! traces against predefined patterns specified in SCML (System Call Matching Language) format.
//!
//! # Modes of Operation
//!
//! ## Online Mode
//!
//! Traces a program in real-time using `strace` and validates each syscall as it occurs:
//!
//! ```bash
//! sctrace patterns.scml -- /bin/ls -la
//! sctrace patterns.scml -- /usr/bin/python3 script.py
//! ```
//!
//! ## Offline Mode
//!
//! Analyzes an existing strace log file:
//!
//! ```bash
//! sctrace patterns.scml --input strace.log
//! strace -o trace.log /bin/ls && sctrace patterns.scml --input trace.log
//! ```
//!
//! # Options
//!
//! - `--quiet` or `-q`: Only output unsupported syscalls (suppresses supported syscalls)
//! - `--input <file>`: Specify input file for offline mode
//!
//! # Output Format
//!
//! - **Supported syscalls**: Printed normally (hidden in quiet mode)
//! - **Unsupported syscalls**: Marked with red `(unsupported)` suffix in normal mode,
//!   or prefixed with `unsupported:` in quiet mode
//! - **Parse errors**: Printed to stderr in red
//!
//! # Examples
//!
//! ## Trace and validate a simple command
//!
//! ```bash
//! sctrace syscalls.scml -- /bin/echo "Hello, World!"
//! ```
//!
//! ## Output only unsupported syscalls
//!
//! ```bash
//! sctrace --quiet syscalls.scml -- /usr/bin/gcc test.c -o test
//! ```
//!
//! ## Process existing trace log
//!
//! ```bash
//! sctrace syscalls.scml --input my_trace.log
//! ```
//!
//! # Exit Codes
//!
//! - `0`: Successful execution
//! - `1`: Error during initialization (invalid parameters, file not found, parse error, etc.)

use std::env;

mod parameter;
mod scml_matcher;
mod scml_parser;
mod strace_parser;
mod trace;

use parameter::Parameters;
use scml_matcher::Matcher;
use scml_parser::Patterns;
use strace_parser::{StraceParseError, Syscall};
use trace::TraceIterator;

/// Handles syscall processing and output formatting.
///
/// This struct encapsulates the logic for processing individual syscalls,
/// matching them against SCML patterns, and formatting output based on whether
/// the syscall is supported or not.
///
/// # Responsibilities
///
/// - Match syscalls against pattern definitions
/// - Format output based on match results and quiet mode settings
struct SyscallHandler<'a> {
    /// Reference to the pattern matcher for validating syscalls
    matcher: &'a Matcher<'a>,
    /// Reference to command-line parameters controlling output behavior
    params: &'a Parameters,
}

impl<'a> SyscallHandler<'a> {
    /// Creates a new SyscallHandler with the given matcher and parameters.
    ///
    /// # Arguments
    ///
    /// * `matcher` - A reference to the pattern matcher used for syscall validation
    /// * `params` - A reference to the command-line parameters for output control
    ///
    /// # Returns
    ///
    /// A new `SyscallHandler` instance ready to process syscalls
    fn new(matcher: &'a Matcher, params: &'a Parameters) -> Self {
        Self { matcher, params }
    }

    /// Processes a single syscall and determines output behavior.
    ///
    /// This method matches the syscall against SCML patterns and prints it with
    /// appropriate formatting based on the match result:
    ///
    /// - **Matched syscalls**: Printed normally unless quiet mode is enabled
    /// - **Unmatched syscalls**: Always printed with unsupported indicator
    ///
    /// # Arguments
    ///
    /// * `syscall` - The parsed syscall to validate and potentially output
    ///
    /// # Output Behavior
    ///
    /// | Mode   | Supported Syscall | Unsupported Syscall |
    /// |--------|-------------------|---------------------|
    /// | Normal | Print line        | Print with red "(unsupported)" |
    /// | Quiet  | No output         | Print with "unsupported:" prefix |
    fn handle(&self, syscall: &Syscall) {
        match self.matcher.match_syscall(syscall) {
            Some(_) => {
                // Syscall is supported, print if not in quiet mode
                if !self.params.quiet() {
                    println!("{}", syscall.original_line());
                }
            }
            None => self.print_unsupported_syscall(syscall),
        }
    }

    /// Prints unsupported syscall with appropriate formatting based on quiet mode.
    ///
    /// The output format varies depending on the quiet mode setting:
    ///
    /// - **Quiet mode**: Prefixes the line with `"unsupported: "`
    /// - **Normal mode**: Appends a red-colored `"(unsupported)"` suffix using ANSI escape codes
    ///
    /// # Arguments
    ///
    /// * `syscall` - The unsupported syscall to print
    fn print_unsupported_syscall(&self, syscall: &Syscall) {
        if self.params.quiet() {
            println!("unsupported: {}", syscall.original_line());
        } else {
            println!("{} \x1b[31m(unsupported)\x1b[0m", syscall.original_line());
        }
    }
}

/// Main entry point for the sctrace tool.
///
/// This function orchestrates the entire syscall tracing and validation process:
///
/// # Workflow
///
/// 1. **Parse Arguments**: Validates and extracts command-line parameters
/// 2. **Load Patterns**: Reads and parses the SCML pattern file
/// 3. **Initialize Tracer**: Creates an appropriate trace iterator (online/offline)
/// 4. **Process Syscalls**: Iterates through trace lines, parsing and validating each syscall
/// 5. **Report Results**: Outputs supported/unsupported syscalls with appropriate formatting
///
/// # Error Handling
///
/// The function exits with code `1` if any of the following errors occur:
/// - Invalid command-line arguments
/// - SCML file not found or malformed
/// - Failed to start tracing (online mode)
/// - Input log file not found (offline mode)
///
/// Parse errors for individual syscall lines are reported to stderr but don't terminate execution.
///
/// # Exit Codes
///
/// - `0` - All operations completed successfully
/// - `1` - Fatal error during initialization or setup
///
/// # Examples
///
/// Running the tool:
///
/// ```bash
/// # Online mode - trace ls command
/// sctrace patterns.scml -- /bin/ls -la
///
/// # Offline mode - analyze existing log
/// sctrace patterns.scml --input trace.log
///
/// # Quiet mode - only show unsupported
/// sctrace --quiet patterns.scml -- /usr/bin/gcc test.c
/// ```
fn main() {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();

    // Initialize parameters from command line arguments
    let params = Parameters::new(args).unwrap_or_else(|e| {
        eprintln!("{}", e);
        std::process::exit(1);
    });

    // Load and parse SCML patterns from file
    let patterns = Patterns::from_scml_file(params.scml_path()).unwrap_or_else(|e| {
        eprintln!("Failed to parse SCML file: {}", e);
        std::process::exit(1);
    });

    println!(
        "Loaded {} syscalls from {}",
        patterns.len(),
        params.scml_path()
    );

    // Create trace iterator for reading syscall traces
    let trace_iter = match params.offline() {
        true => TraceIterator::new_offline(params.input_path()),
        false => TraceIterator::new_online(params.program_path(), params.program_args()),
    }
    .unwrap_or_else(|e| {
        eprintln!("Failed to create trace iterator: {}", e);
        std::process::exit(1);
    });

    // Initialize matcher and handler
    let scml_matcher = Matcher::new(patterns);
    let handler = SyscallHandler::new(&scml_matcher, &params);

    // Process each line from the trace
    for line in trace_iter {
        match Syscall::fetch(line) {
            Ok(line) => match Syscall::parse(&line) {
                Ok(syscall) => {
                    handler.handle(&syscall);
                }
                Err(e) => {
                    eprintln!("Processing line: {}", line);
                    eprintln!("\x1b[31mStrace Parse Error: {}\x1b[0m", e);
                }
            },
            Err(
                StraceParseError::BlockedLine
                | StraceParseError::SignalLine
                | StraceParseError::ExitLine,
            ) => {
                // Ignore blocked, signal and exit lines
                continue;
            }
            Err(e) => {
                panic!("Unexpected error: {}", e);
            }
        }
    }
}
