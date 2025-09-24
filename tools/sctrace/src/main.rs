// SPDX-License-Identifier: MPL-2.0

mod parameter;

use std::env;

use parameter::Parameters;
use sctrace::{CliReporterBuilder, Patterns, SctraceBuilder, StraceLogStream};

fn run() -> Result<(), String> {
    let args: Vec<String> = env::args().collect();
    let params = Parameters::new(args)?;

    let sctrace = SctraceBuilder::new()
        .patterns(Patterns::from_scml_files(params.scml_paths())?)
        .strace(if params.offline() {
            StraceLogStream::open_file(params.input_path())
        } else {
            StraceLogStream::run_cmd(params.program_path(), params.program_args())
        }?)
        .reporter(CliReporterBuilder::new().set_quiet(params.quiet()).build())
        .build();

    let _ = sctrace.run();
    Ok(())
}

fn main() {
    if let Err(err) = run() {
        eprintln!("{}", err);
        std::process::exit(1);
    }
}
