// SPDX-License-Identifier: MPL-2.0

use std::{error::Error, fmt};

use clap::{Arg, ArgAction, Command};

/// Holds parsed command-line parameters for the sctrace tool.
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

#[derive(Debug)]
pub struct ParameterError {
    message: String,
}

impl fmt::Display for ParameterError {
    fn fmt(&self, f: &mut fmt::Formatter) -> fmt::Result {
        write!(f, "{}", self.message)
    }
}

impl From<ParameterError> for String {
    fn from(err: ParameterError) -> Self {
        err.to_string()
    }
}

impl Error for ParameterError {}

impl Parameters {
    /// Creates a new Parameters instance from command line arguments.
    ///
    /// ```text
    /// sctrace <SCML_FILE> [--input FILE] [--quiet] [-- PROGRAM] [PROGRAM_ARGS...]
    /// ```
    pub fn new(args: Vec<String>) -> Result<Self, ParameterError> {
        let app = Command::new("sctrace")
            .about("Syscall Compatibility Tracer (sctrace) - trace and validate syscalls against SCML patterns")
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

        let matches = app.try_get_matches_from(args).map_err(|e| ParameterError {
            message: e.to_string(),
        })?;
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
            .map(|values| values.cloned().collect())
            .unwrap_or_default();

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

    pub fn scml_path(&self) -> &str {
        &self.scml_path
    }

    pub fn input_path(&self) -> &str {
        &self.input_path
    }

    pub fn quiet(&self) -> bool {
        self.quiet
    }

    pub fn offline(&self) -> bool {
        !self.input_path.is_empty()
    }

    pub fn program_path(&self) -> &str {
        &self.program_path
    }

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
