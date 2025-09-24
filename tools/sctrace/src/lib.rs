// SPDX-License-Identifier: MPL-2.0

//! # Sctrace - Syscall Compatibility Tracer
//!
//! `sctrace` is a library for analyzing `strace` logs against SCML (Syscall Matching Language)
//! patterns to determine syscall compatibility. It provides APIs for parsing `strace` output,
//! matching syscalls against pattern specifications, and reporting compatibility results.
//!
//! ## Examples
//!
//! ```no_run
//! use sctrace::{CliReporterBuilder, Patterns, SctraceBuilder, StraceLogStream};
//!
//! fn main() -> Result<(), String> {
//!     let scml_files = vec!["patterns1.scml", "patterns2.scml"];
//!
//!     let sctrace = SctraceBuilder::new()
//!         .strace(StraceLogStream::open_file("strace.log")?)
//!         .patterns(Patterns::from_scml_files(&scml_files)?)
//!         .reporter(CliReporterBuilder::new().quiet().build())
//!         .build();
//!
//!     let _ = sctrace.run();
//!     Ok(())
//! }
//! ```
mod scml_matcher;
mod scml_parser;
mod strace_parser;
mod trace;

use std::io::Lines;

use scml_matcher::Matcher;
pub use scml_parser::Patterns;
use strace_parser::{StraceParseError, Syscall};
pub use trace::StraceLogStream;

/// Builder for creating an `Sctrace` instance.
pub struct SctraceBuilder<'a> {
    log_stream: Option<StraceLogStream>,
    patterns: Option<Patterns<'a>>,
    reporter: Option<CliReporter>,
}

impl<'a> SctraceBuilder<'a> {
    /// Creates a new `SctraceBuilder` instance.
    pub fn new() -> Self {
        Self {
            log_stream: None,
            patterns: None,
            reporter: None,
        }
    }

    /// Sets the strace log stream for the tracer.
    pub fn strace(mut self, stream: StraceLogStream) -> Self {
        self.log_stream = Some(stream);
        self
    }

    /// Sets the SCML patterns for the tracer.
    pub fn patterns(mut self, patterns: Patterns<'a>) -> Self {
        self.patterns = Some(patterns);
        self
    }

    /// Sets the reporter for the tracer.
    pub fn reporter(mut self, reporter: CliReporter) -> Self {
        self.reporter = Some(reporter);
        self
    }

    /// Builds the `Sctrace` instance with the specified components.
    pub fn build(self) -> Sctrace<'a> {
        Sctrace {
            strace_iter: self.log_stream.expect("`log_stream` is required").lines(),
            patterns: self.patterns.expect("`patterns` is required"),
            reporter: self.reporter.expect("`reporter` is required"),
        }
    }
}

impl Default for SctraceBuilder<'_> {
    fn default() -> Self {
        Self::new()
    }
}

/// The high-level API of syscall compatibility tracer.
pub struct Sctrace<'a> {
    strace_iter: Lines<StraceLogStream>,
    patterns: Patterns<'a>,
    reporter: CliReporter,
}

impl Sctrace<'_> {
    /// Runs the syscall trace analysis.
    pub fn run(mut self) -> Result<Option<Vec<String>>, String> {
        let matcher = Matcher::new(self.patterns.clone());

        for line_result in &mut self.strace_iter {
            let line = line_result.map_err(|e| format!("Failed to read trace line: {}", e))?;

            match Syscall::fetch(line) {
                Ok(line) => match Syscall::parse(&line) {
                    Ok(syscall) => {
                        if matcher.match_syscall(&syscall).is_some() {
                            self.reporter.report_supported(&syscall);
                        } else {
                            self.reporter.report_unsupported(&syscall);
                        }
                    }
                    Err(_) => {
                        self.reporter.report_parse_error(&line);
                    }
                },
                Err(e) => {
                    match e {
                        StraceParseError::BlockedLine
                        | StraceParseError::SignalLine
                        | StraceParseError::ExitLine
                        | StraceParseError::EmptyLine => {
                            // Ignore blocked, signal, exit and empty lines
                            continue;
                        }
                        _ => {
                            panic!("Unexpected error: {}", e);
                        }
                    }
                }
            }
        }

        Ok(self.reporter.outputs())
    }
}

/// Builder for creating a `CliReporter` with customizable settings.
pub struct CliReporterBuilder {
    quiet: bool,
    collect: bool,
}

impl CliReporterBuilder {
    /// Creates a new `CliReporterBuilder` instance.
    pub fn new() -> Self {
        Self {
            quiet: false,
            collect: false,
        }
    }

    /// Enables quiet mode, suppressing supported syscall output.
    pub fn quiet(mut self) -> Self {
        self.quiet = true;
        self
    }

    /// Sets quiet mode.
    pub fn set_quiet(mut self, quiet: bool) -> Self {
        self.quiet = quiet;
        self
    }

    /// Enables collection of output strings instead of printing to stdout/stderr.
    pub fn collect(mut self) -> Self {
        self.collect = true;
        self
    }

    /// Sets collection mode.
    pub fn set_collect(mut self, collect: bool) -> Self {
        self.collect = collect;
        self
    }

    /// Builds the `CliReporter` instance with the specified settings.
    pub fn build(self) -> CliReporter {
        CliReporter::new(
            self.quiet,
            if self.collect { Some(Vec::new()) } else { None },
        )
    }
}

impl Default for CliReporterBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Reporter for outputting syscall trace analysis results.
pub struct CliReporter {
    quiet: bool,
    outputs: Option<Vec<String>>,
}

macro_rules! report {
    ($vec:expr, err: $($arg:tt)*) => {
        if let Some(ref mut vec) = $vec {
            (&mut *vec).push(format!($($arg)*));
        } else {
            eprintln!($($arg)*);
        }
    };
    ($vec:expr, $($arg:tt)*) => {
        if let Some(ref mut vec) = $vec {
            (&mut *vec).push(format!($($arg)*));
        } else {
            println!($($arg)*);
        }
    };
}

impl CliReporter {
    fn new(quiet: bool, outputs: Option<Vec<String>>) -> Self {
        Self { quiet, outputs }
    }

    fn report_supported(&mut self, syscall: &Syscall) {
        if !self.quiet {
            report!(self.outputs, "{}", syscall.original_line());
        }
    }

    fn report_unsupported(&mut self, syscall: &Syscall) {
        if self.quiet {
            report!(
                self.outputs,
                err: "Unsupported syscall: {}",
                syscall.original_line()
            );
        } else {
            report!(
                self.outputs,
                err: "{} \x1b[31m(unsupported)\x1b[0m",
                syscall.original_line()
            );
        }
    }

    fn report_parse_error(&mut self, line: &str) {
        report!(self.outputs, err: "Strace Parse Error: {}", line);
    }

    fn outputs(self) -> Option<Vec<String>> {
        self.outputs
    }
}
