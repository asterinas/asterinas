// SPDX-License-Identifier: MPL-2.0

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

/// Handles syscall processing and output formatting
struct SyscallHandler<'a> {
    matcher: &'a Matcher,
    params: &'a Parameters,
}

impl<'a> SyscallHandler<'a> {
    /// Creates a new SyscallHandler with the given matcher and parameters
    fn new(matcher: &'a Matcher, params: &'a Parameters) -> Self {
        Self { matcher, params }
    }

    /// Processes a single syscall and determines output behavior
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

    /// Prints unsupported syscall with appropriate formatting based on quiet mode
    fn print_unsupported_syscall(&self, syscall: &Syscall) {
        if self.params.quiet() {
            println!("unsupported: {}", syscall.original_line());
        } else {
            println!("{} \x1b[31m(unsupported)\x1b[0m", syscall.original_line());
        }
    }
}

fn main() {
    // Parse command line arguments
    let args: Vec<String> = env::args().collect();

    // Initialize parameters from command line arguments
    let params = Parameters::new(args).unwrap_or_else(|e| {
        eprintln!("Failed to parse parameters: {}", e);
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
    let trace_iter = TraceIterator::new(&params).unwrap_or_else(|e| {
        eprintln!("Failed to create trace iterator: {}", e);
        std::process::exit(1);
    });

    // Initialize matcher and handler
    let scml_matcher = Matcher::new(patterns);
    let handler = SyscallHandler::new(&scml_matcher, &params);

    // Process each line from the trace
    for line in trace_iter {
        match Syscall::parse(&line) {
            Ok(syscall) => {
                handler.handle(&syscall);
            }
            Err(
                StraceParseError::BlockedLine
                | StraceParseError::SignalLine
                | StraceParseError::ExitLine,
            ) => {
                // Ignore blocked, signal and exit lines
                continue;
            }
            Err(e) => {
                eprintln!("Processing line: {}", line);
                eprintln!("\x1b[31mStrace Parse Error: {}\x1b[0m", e);
            }
        }
    }
}
