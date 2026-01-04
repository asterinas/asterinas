// SPDX-License-Identifier: MPL-2.0

use std::{error::Error, fmt};

use clap::{Arg, ArgAction, Command};

/// Holds parsed command-line parameters for the sctrace tool.
#[derive(Debug)]
pub(crate) struct Parameters {
    /// Path to the SCML pattern files
    scml_paths: Vec<String>,
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
pub(crate) struct ParameterError {
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
    /// sctrace <SCML_PATH1> [SCML_PATH2...] [--input FILE] [--quiet] [-- PROGRAM] [PROGRAM_ARGS...]
    /// ```
    pub(crate) fn new(args: Vec<String>) -> Result<Self, ParameterError> {
        let app = Command::new("sctrace")
            .about("Syscall Compatibility Tracer (sctrace) - trace and validate syscalls against SCML patterns")
            .version(env!("CARGO_PKG_VERSION"))
            .after_help("EXAMPLES:\n  \
                Offline mode: sctrace patterns.scml --input trace.log\n  \
                Online mode:  sctrace patterns.scml -- /bin/ls -la")
            .arg(
                Arg::new("scml_paths")
                    .help("Path(s) to SCML file(s)")
                    .required(true)
                    .num_args(1..)
                    .value_name("SCML_PATHS"),
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
            .arg(
                Arg::new("program")
                    .help("Program to execute and its arguments (use -- to separate)")
                    .num_args(0..)
                    .value_name("PROGRAM")
                    .last(true)
                    .allow_hyphen_values(true),
            );

        let matches = app.try_get_matches_from(args).map_err(|e| ParameterError {
            message: e.to_string(),
        })?;

        let scml_paths: Vec<String> = matches
            .get_many::<String>("scml_paths")
            .map(|values| values.cloned().collect())
            .unwrap_or_default();

        let input_path = matches
            .get_one::<String>("input")
            .unwrap_or(&String::new())
            .clone();
        let quiet = matches.get_flag("quiet");

        let mut program_iter = matches
            .get_many::<String>("program")
            .map(|values| values.cloned())
            .into_iter()
            .flatten();

        let program_path = program_iter.next().unwrap_or_default();
        let program_args: Vec<String> = program_iter.collect();

        let has_input = !input_path.is_empty();
        let has_program = !program_path.is_empty();

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
            scml_paths,
            input_path,
            quiet,
            program_path,
            program_args,
        })
    }

    pub(crate) fn scml_paths(&self) -> &Vec<String> {
        &self.scml_paths
    }

    pub(crate) fn input_path(&self) -> &str {
        &self.input_path
    }

    pub(crate) fn quiet(&self) -> bool {
        self.quiet
    }

    pub(crate) fn offline(&self) -> bool {
        !self.input_path.is_empty()
    }

    pub(crate) fn program_path(&self) -> &str {
        &self.program_path
    }

    pub(crate) fn program_args(&self) -> Vec<&str> {
        self.program_args.iter().map(String::as_str).collect()
    }
}

#[cfg(test)]
mod tests {
    use std::vec;

    use super::*;

    fn to_string_vec(args: &[&str]) -> Vec<String> {
        args.iter().map(|s| s.to_string()).collect()
    }

    #[test]
    fn test_new_offline_mode() {
        let args = to_string_vec(&["sctrace", "test.scml", "--input", "input.txt"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_paths, vec!["test.scml"]);
        assert_eq!(params.input_path(), "input.txt");
        assert!(!params.quiet());
        assert_eq!(params.program_path(), "");
        assert!(params.program_args().is_empty());
        assert!(params.offline());
    }

    #[test]
    fn test_new_online_mode() {
        let args = to_string_vec(&["sctrace", "test.scml", "--", "my_program", "arg1", "arg2"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_paths, vec!["test.scml"]);
        assert_eq!(params.input_path(), "");
        assert!(!params.quiet());
        assert_eq!(params.program_path(), "my_program");
        assert_eq!(params.program_args(), &["arg1", "arg2"]);
        assert!(!params.offline());
    }

    #[test]
    fn test_new_online_mode_no_args() {
        let args = to_string_vec(&["sctrace", "test.scml", "--", "my_program"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_paths, vec!["test.scml"]);
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
        let params = Parameters::new(args);
        assert!(params.is_err());
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
    fn test_many_scml_paths() {
        let args = to_string_vec(&["sctrace", "--input", "in.txt", "a.scml", "b.scml", "c.scml"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_paths, vec!["a.scml", "b.scml", "c.scml"]);

        let args = to_string_vec(&["sctrace", "a.scml", "b.scml", "c.scml", "--", "prog", "arg"]);
        let params = Parameters::new(args).unwrap();
        assert_eq!(params.scml_paths, vec!["a.scml", "b.scml", "c.scml"]);
        assert_eq!(params.program_path(), "prog");
        assert_eq!(params.program_args(), &["arg"]);
    }

    #[test]
    fn test_new_both_input_and_program() {
        let args = to_string_vec(&[
            "sctrace", "--input", "in.txt", "a.scml", "b.scml", "c.scml", "--", "prog",
        ]);
        let result = Parameters::new(args);
        assert!(result.is_err());
        assert_eq!(
            result.err().unwrap().to_string(),
            "Cannot specify both --input and program arguments"
        );
    }
}
