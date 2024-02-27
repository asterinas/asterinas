// Licensed under the Apache License, Version 2.0 or the MIT License.
// Copyright (C) 2023-2024 Ant Group.

// This implementation is from rust clippy. We modified the code.

use std::env;
use std::path::PathBuf;
use std::process::{self, Command};

const CARGO_COMPONENT_HELP: &str = r#"Checks whether a package violates access control policy.
Usage:
    cargo component [options]
Common options:
    audit   
    check   
"#;

fn show_help() {
    println!("{CARGO_COMPONENT_HELP}");
}

fn show_version() {
    let version_info = rustc_tools_util::get_version_info!();
    println!("{version_info}");
}

pub fn main() {
    // Check for version and help flags even when invoked as 'cargo-component'
    if env::args().any(|a| a == "--help" || a == "-h") {
        show_help();
        return;
    }

    if env::args().any(|a| a == "--version" || a == "-V") {
        show_version();
        return;
    }

    if let Err(code) = process(env::args().skip(2)) {
        process::exit(code);
    }
}

struct ComponentCmd {
    cargo_subcommand: &'static str,
    args: Vec<String>,
    component_args: Vec<String>,
}

impl ComponentCmd {
    fn new<I>(mut old_args: I) -> Self
    where
        I: Iterator<Item = String>,
    {
        let cargo_subcommand = "check";
        let mut args = vec![];
        let mut component_args: Vec<String> = vec![];

        for arg in old_args.by_ref() {
            match arg.as_str() {
                "check" => {
                    component_args.push("check".into());
                    continue;
                }
                "audit" => {
                    component_args.push("audit".into());
                    continue;
                }
                "--" => break,
                _ => {}
            }
            args.push(arg);
        }

        component_args.append(&mut (old_args.collect()));

        Self {
            cargo_subcommand,
            args,
            component_args,
        }
    }

    fn path() -> PathBuf {
        let mut path = env::current_exe()
            .expect("current executable path invalid")
            .with_file_name("component-driver");

        if cfg!(windows) {
            path.set_extension("exe");
        }

        path
    }

    fn into_std_cmd(self) -> Command {
        let mut cmd = Command::new("cargo");
        let component_args: String = self
            .component_args
            .iter()
            .map(|arg| format!("{arg}"))
            .collect();
        cmd.env("RUSTC_WORKSPACE_WRAPPER", Self::path())
            .env("COMPONENT_ARGS", component_args)
            .env("COMPONENT_CONFIG_DIR", std::env::current_dir().unwrap())
            .arg(self.cargo_subcommand)
            .args(&self.args);

        cmd
    }
}

fn process<I>(old_args: I) -> Result<(), i32>
where
    I: Iterator<Item = String>,
{
    let cmd = ComponentCmd::new(old_args);

    let mut cmd = cmd.into_std_cmd();

    let exit_status = cmd
        .spawn()
        .expect("could not run cargo")
        .wait()
        .expect("failed to wait for cargo?");

    if exit_status.success() {
        Ok(())
    } else {
        Err(exit_status.code().unwrap_or(-1))
    }
}
