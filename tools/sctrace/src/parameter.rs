// SPDX-License-Identifier: MPL-2.0

//! Command-line parameter parsing and validation for the sctrace tool.
//!
//! This module provides the [`Parameters`] struct for parsing and storing
//! command-line arguments, supporting both online (trace a program) and
//! offline (analyze a log file) modes.

use std::{error::Error, fmt};

use clap::{Arg, ArgAction, Command};

/// Holds parsed command-line parameters for the sctrace tool.
///
/// This struct stores all configuration options parsed from command-line arguments,
/// including the SCML pattern file path, input file path (for offline mode),
/// program information (for online mode), and various flags.
///
/// # Examples
///
/// ```
/// use sctrace::Parameters;
///
/// let args = vec!["sctrace", "patterns.scml", "--", "/bin/ls", "-la"]
///     .iter()
///     .map(|s| s.to_string())
///     .collect();
/// let params = Parameters::new(args).unwrap();
/// ```
#[derive(Debug)]
pub struct Parameters {
    /// Path to the SCML pattern file
    scml_path: String,
    /// Path to the input strace log file (offline mode)
    input_path: String,
    /// Whether quiet mode is enabled
    quiet: bool,
    /// Path to the program to trace (online mode)
    program_path: String,
    /// Arguments to pass to the program (online mode)
    program_args: Vec<String>,
}

/// Error type for parameter parsing failures.
///
/// This error is returned when command-line arguments cannot be parsed
/// or when invalid combinations of arguments are provided.
#[derive(Debug)]
pub struct ParameterError {
    /// Human-readable error message
    message: String,
}

impl fmt::Display for ParameterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl Error for ParameterError {}

impl Parameters {
    /// Creates a new Parameters instance from command line arguments.
    ///
    /// # Command-Line Syntax
    ///
    /// ```text
    /// sctrace <SCML_FILE> [--input FILE] [--quiet] [PROGRAM] [PROGRAM_ARGS...]
    /// ```
    ///
    /// # Arguments
    ///
    /// * `<SCML_FILE>` - Path to the SCML pattern file (required)
    /// * `--input FILE` - Path to strace log file for offline mode (optional)
    /// * `--quiet` - Enable quiet mode, only show unsupported syscalls (optional)
    /// * `PROGRAM` - Program to execute and trace in online mode (optional)
    /// * `PROGRAM_ARGS` - Arguments to pass to the program (optional)
    ///
    /// # Modes
    ///
    /// The tool operates in one of two mutually exclusive modes:
    ///
    /// - **Offline mode**: Use `--input FILE` to analyze an existing strace log
    /// - **Online mode**: Specify `PROGRAM [ARGS...]` to trace a running program
    ///
    /// # Returns
    ///
    /// Returns `Ok(Parameters)` if parsing succeeds, or an error if:
    /// - Required arguments are missing
    /// - Both `--input` and program are specified
    /// - Neither `--input` nor program are specified
    /// - Invalid argument format is provided
    ///
    /// # Examples
    ///
    /// Offline mode:
    /// ```
    /// # use sctrace::Parameters;
    /// let args = vec!["sctrace", "patterns.scml", "--input", "trace.log"]
    ///     .iter().map(|s| s.to_string()).collect();
    /// let params = Parameters::new(args).unwrap();
    /// assert!(params.offline());
    /// ```
    ///
    /// Online mode:
    /// ```
    /// # use sctrace::Parameters;
    /// let args = vec!["sctrace", "patterns.scml", "/bin/ls", "-la"]
    ///     .iter().map(|s| s.to_string()).collect();
    /// let params = Parameters::new(args).unwrap();
    /// assert!(!params.offline());
    /// ```
    pub fn new(args: Vec<String>) -> Result<Self, ParameterError> {
        let app = Command::new("sctrace")
            .about("Syscall-Compliance-Trace (sctrace) - trace and validate syscalls against SCML patterns")
            .after_help("EXAMPLES:\n  \
                Offline mode: sctrace patterns.scml --input trace.log\n  \
                Online mode:  sctrace patterns.scml -- /bin/ls -la")
            .arg(
                Arg::new("scml_file")
                    .help("SCML file path")
                    .required(true)
                    .index(1),
            )
            .arg(
                Arg::new("input")
                    .long("input")
                    .help("Input file path")
                    .value_name("FILE")
                    .action(ArgAction::Set),
            )
            .arg(
                Arg::new("quiet")
                    .long("quiet")
                    .help("Enable quiet mode")
                    .action(ArgAction::SetTrue),
            )
            .arg(Arg::new("program").help("Program to execute").index(2))
            .arg(
                Arg::new("program_args")
                    .help("Program arguments")
                    .action(ArgAction::Append)
                    .index(3),
            );

        let matches = match app.try_get_matches_from(args) {
            Ok(matches) => matches,
            Err(e) => {
                return Err(ParameterError {
                    message: format!("{}", e),
                });
            }
        };

        let scml_path = matches.get_one::<String>("scml_file").unwrap().clone();
        let input_path = matches
            .get_one::<String>("input")
            .unwrap_or(&String::new())
            .clone();
        let quiet = matches.get_flag("quiet");
        let program_path = matches
            .get_one::<String>("program")
            .unwrap_or(&String::new())
            .clone();
        let program_args: Vec<String> = matches
            .get_many::<String>("program_args")
            .map(|values| values.map(|v| v.clone()).collect())
            .unwrap_or_default();

        // Validation logic
        let has_input = !input_path.is_empty();
        let has_program = !program_path.is_empty() || !program_args.is_empty();

        if has_input && has_program {
            return Err(ParameterError {
                message: "Cannot specify both --input and program arguments".to_string(),
            });
        }

        if !has_input && !has_program {
            return Err(ParameterError {
                message: "Must specify either --input or program arguments".to_string(),
            });
        }

        Ok(Parameters {
            scml_path,
            input_path,
            quiet,
            program_path,
            program_args,
        })
    }

    /// Returns the path to the SCML pattern file.
    ///
    /// # Examples
    ///
    /// ```
    /// # use sctrace::Parameters;
    /// # let params = Parameters::new(vec!["sctrace", "patterns.scml", "--input", "log.txt"]
    /// #     .iter().map(|s| s.to_string()).collect()).unwrap();
    /// assert_eq!(params.scml_path(), "patterns.scml");
    /// ```
    pub fn scml_path(&self) -> &str {
        &self.scml_path
    }

    /// Returns the path to the input strace log file.
    ///
    /// Returns an empty string if running in online mode.
    ///
    /// # Examples
    ///
    /// ```text
    /// # use sctrace::Parameters;
    /// # let params = Parameters::new(vec!["sctrace", "patterns.scml", "--input", "log.txt"]
    /// #     .iter().map(|s| s.to_string()).collect()).unwrap();
    /// assert_eq!(params.input_path(), "log.txt");
    /// ```
    pub fn input_path(&self) -> &str {
        &self.input_path
    }

    /// Returns whether quiet mode is enabled.
    ///
    /// In quiet mode, only unsupported syscalls are printed.
    ///
    /// # Examples
    ///
    /// ```
    /// # use sctrace::Parameters;
    /// # let params = Parameters::new(vec!["sctrace", "patterns.scml", "--quiet", "--input", "log.txt"]
    /// #     .iter().map(|s| s.to_string()).collect()).unwrap();
    /// assert!(params.quiet());
    /// ```
    pub fn quiet(&self) -> bool {
        self.quiet
    }

    /// Returns whether the tool is running in offline mode.
    ///
    /// Returns `true` when analyzing an existing strace log file,
    /// `false` when tracing a live program.
    ///
    /// # Examples
    ///
    /// ```
    /// # use sctrace::Parameters;
    /// # let params = Parameters::new(vec!["sctrace", "patterns.scml", "--input", "log.txt"]
    /// #     .iter().map(|s| s.to_string()).collect()).unwrap();
    /// assert!(params.offline());
    /// ```
    pub fn offline(&self) -> bool {
        !self.input_path.is_empty()
    }

    /// Returns the path to the program to trace.
    ///
    /// Returns an empty string if running in offline mode.
    ///
    /// # Examples
    ///
    /// ```
    /// # use sctrace::Parameters;
    /// # let params = Parameters::new(vec!["sctrace", "patterns.scml", "/bin/ls"]
    /// #     .iter().map(|s| s.to_string()).collect()).unwrap();
    /// assert_eq!(params.program_path(), "/bin/ls");
    /// ```
    pub fn program_path(&self) -> &str {
        &self.program_path
    }

    /// Returns the arguments to pass to the traced program.
    ///
    /// Returns an empty vector if running in offline mode or if no arguments were provided.
    ///
    /// # Examples
    ///
    /// ```
    /// # use sctrace::Parameters;
    /// # let params = Parameters::new(vec!["sctrace", "patterns.scml", "/bin/ls", "-la", "-h"]
    /// #     .iter().map(|s| s.to_string()).collect()).unwrap();
    /// assert_eq!(params.program_args(), vec!["-la", "-h"]);
    /// ```
    pub fn program_args(&self) -> Vec<&str> {
        self.program_args.iter().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn to_string_vec(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_new_offline_mode() {
        let args = to_string_vec(&["sctrace", "test.scml", "--input", "input.txt"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_path(), "test.scml");
        assert_eq!(params.input_path(), "input.txt");
        assert_eq!(params.quiet(), false);
        assert_eq!(params.program_path(), "");
        assert!(params.program_args().is_empty());
        assert!(params.offline());
    }

    #[test]
    fn test_new_online_mode() {
        let args = to_string_vec(&["sctrace", "test.scml", "my_program", "arg1", "arg2"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_path(), "test.scml");
        assert_eq!(params.input_path(), "");
        assert_eq!(params.quiet(), false);
        assert_eq!(params.program_path(), "my_program");
        assert_eq!(params.program_args(), &["arg1", "arg2"]);
        assert!(!params.offline());
    }

    #[test]
    fn test_new_online_mode_no_args() {
        let args = to_string_vec(&["sctrace", "test.scml", "my_program"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_path(), "test.scml");
        assert_eq!(params.program_path(), "my_program");
        assert!(params.program_args().is_empty());
        assert!(!params.offline());
    }

    #[test]
    fn test_new_quiet_mode() {
        let args = to_string_vec(&["sctrace", "test.scml", "--quiet", "--input", "input.txt"]);
        let params = Parameters::new(args).unwrap();
        assert!(params.quiet());
    }

    #[test]
    fn test_new_missing_scml_file() {
        let args = to_string_vec(&["sctrace", "--input", "input.txt"]);
        let result = Parameters::new(args);
        assert!(result.is_err());
    }

    #[test]
    fn test_new_missing_input_and_program() {
        let args = to_string_vec(&["sctrace", "test.scml"]);
        let result = Parameters::new(args);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "Must specify either --input or program arguments"
        );
    }

    #[test]
    fn test_new_both_input_and_program() {
        let args = to_string_vec(&["sctrace", "test.scml", "--input", "in.txt", "prog"]);
        let result = Parameters::new(args);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "Cannot specify both --input and program arguments"
        );
    }
}
