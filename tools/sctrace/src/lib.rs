// SPDX-License-Identifier: MPL-2.0

//! A library for checking syscall traces against SCML (System Call Matching Language) patterns.
//!
//! This library provides functionality to validate syscall traces either from live program
//! execution or from strace log files against predefined patterns specified in SCML format.
//!
//! # Overview
//!
//! The library offers two main functions:
//! - [`check_program`] - Traces a running program in real-time
//! - [`check_logfile`] - Analyzes an existing strace log file
//!
//! Both functions return a vector of error messages for syscalls that don't match the patterns
//! or cannot be parsed. An empty vector indicates all syscalls are supported.
//!
//! # Examples
//!
//! ## Checking a running program
//!
//! ```text
//! use sctrace::check_program;
//!
//! let results = check_program(
//!     "patterns.scml",
//!     "/bin/ls",
//!     vec!["-la"]
//! ).expect("Failed to check program");
//!
//! if results.is_empty() {
//!     println!("All syscalls are supported!");
//! } else {
//!     for error in results {
//!         println!("Issue: {}", error);
//!     }
//! }
//! ```
//!
//! ## Checking a log file
//!
//! ```text
//! use sctrace::check_logfile;
//!
//! let results = check_logfile(
//!     "patterns.scml",
//!     "strace.log"
//! ).expect("Failed to check log file");
//!
//! for error in results {
//!     println!("Issue: {}", error);
//! }
//! ```

mod scml_matcher;
mod scml_parser;
mod strace_parser;
mod trace;

use scml_matcher::Matcher;
use scml_parser::Patterns;
use strace_parser::{StraceParseError, Syscall};
use trace::TraceIterator;

macro_rules! push_result {
    ($vec:expr, $($arg:tt)*) => {
        (&mut $vec).push(format!($($arg)*));
    };
}

/// Checks a program's syscall trace against SCML patterns in real-time.
///
/// This function spawns the specified program and traces its syscalls using strace,
/// comparing them against the patterns defined in the SCML file. The program runs
/// to completion while being traced.
///
/// # Arguments
///
/// * `scml_path` - Path to the SCML pattern file containing syscall patterns
/// * `program_path` - Path to the executable to trace
/// * `program_args` - Command-line arguments to pass to the program
///
/// # Returns
///
/// Returns `Ok(Vec<String>)` containing error messages for:
/// - Syscalls that don't match any pattern in the SCML file
/// - Lines that cannot be parsed as valid syscalls
///
/// An empty vector indicates all syscalls matched the patterns successfully.
///
/// # Errors
///
/// Returns `Err(String)` if:
/// - The SCML file cannot be read or parsed
/// - The program cannot be started for tracing
/// - Strace fails to attach to the process
///
/// # Examples
///
/// ```text
/// use sctrace::check_program;
///
/// // Trace the echo command
/// let results = check_program(
///     "syscalls.scml",
///     "/bin/echo",
///     vec!["hello", "world"]
/// ).expect("Failed to trace program");
///
/// // Check results
/// if results.is_empty() {
///     println!("✓ All syscalls are supported");
/// } else {
///     println!("✗ Found {} issues:", results.len());
///     for error in results {
///         println!("  - {}", error);
///     }
/// }
/// ```
pub fn check_program(
    scml_path: &str,
    program_path: &str,
    program_args: Vec<&str>,
) -> Result<Vec<String>, String> {
    let trace_iter = TraceIterator::new_online(program_path, program_args)
        .map_err(|e| format!("Failed to start tracing: {}", e))?;

    check(scml_path, trace_iter)
}

/// Checks a strace log file against SCML patterns.
///
/// This function reads an existing strace output file and validates each syscall
/// entry against the patterns defined in the SCML file. The log file should be
/// in the standard strace format.
///
/// # Arguments
///
/// * `scml_path` - Path to the SCML pattern file containing syscall patterns
/// * `input_path` - Path to the strace log file to analyze
///
/// # Returns
///
/// Returns `Ok(Vec<String>)` containing error messages for:
/// - Syscalls that don't match any pattern in the SCML file
/// - Log lines that cannot be parsed as valid syscalls
///
/// An empty vector indicates all syscalls matched the patterns successfully.
///
/// # Errors
///
/// Returns `Err(String)` if:
/// - The SCML file cannot be read or parsed
/// - The input log file cannot be opened or read
///
/// # Examples
///
/// ```text
/// use sctrace::check_logfile;
///
/// // Analyze a previously captured strace log
/// let results = check_logfile(
///     "syscalls.scml",
///     "trace_output.log"
/// ).expect("Failed to analyze log file");
///
/// // Report results
/// println!("Analyzed log file:");
/// if results.is_empty() {
///     println!("  ✓ All syscalls are supported");
/// } else {
///     println!("  ✗ Found {} unsupported syscalls", results.len());
///     for error in &results[..5.min(results.len())] {
///         println!("    - {}", error);
///     }
///     if results.len() > 5 {
///         println!("    ... and {} more", results.len() - 5);
///     }
/// }
/// ```
pub fn check_logfile(scml_path: &str, input_path: &str) -> Result<Vec<String>, String> {
    let trace_iter = TraceIterator::new_offline(input_path)
        .map_err(|e| format!("Failed to open input file: {}", e))?;

    check(scml_path, trace_iter)
}

/// Internal function that performs the actual checking of syscall traces.
///
/// This function:
/// 1. Parses the SCML patterns from the specified file
/// 2. Creates a matcher for pattern matching
/// 3. Iterates through trace lines and validates each syscall
/// 4. Collects errors for unsupported or unparsable syscalls
///
/// # Arguments
///
/// * `scml_path` - Path to the SCML pattern file
/// * `trace_iter` - Iterator providing syscall trace lines
///
/// # Returns
///
/// Returns `Ok(Vec<String>)` with error messages for problematic syscalls,
/// or `Err(String)` if SCML parsing or matcher initialization fails.
fn check(scml_path: &str, trace_iter: impl Iterator<Item = String>) -> Result<Vec<String>, String> {
    let mut result_strings = Vec::new();

    // Parse SCML patterns
    let patterns = Patterns::from_scml_file(scml_path)
        .map_err(|e| format!("Failed to parse SCML file: {}", e))?;

    println!("Loaded {} syscalls from {}", patterns.len(), scml_path);

    // Create matcher
    let matcher = Matcher::new(patterns);

    for line in trace_iter {
        match Syscall::fetch(line) {
            Ok(line) => match Syscall::parse(&line) {
                Ok(syscall) => {
                    if matcher.match_syscall(&syscall).is_none() {
                        push_result!(
                            result_strings,
                            "Unsupported syscall: {}",
                            syscall.original_line()
                        );
                    }
                }
                Err(_) => {
                    push_result!(result_strings, "Strace Parse Error: {}", line);
                }
            },
            Err(e) => {
                match e {
                    StraceParseError::BlockedLine
                    | StraceParseError::SignalLine
                    | StraceParseError::ExitLine => {
                        // Ignore blocked, signal and exit lines
                        continue;
                    }
                    _ => {
                        panic!("Unexpected error: {}", e);
                    }
                }
            }
        }
    }

    Ok(result_strings)
}
