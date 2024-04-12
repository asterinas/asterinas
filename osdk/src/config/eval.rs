// SPDX-License-Identifier: MPL-2.0

//! The module implementing the evaluation feature.

use std::{io, process};

pub type Vars = Vec<(String, String)>;

/// This function is used to evaluate the string using the host's shell recursively
/// in order.
pub fn eval(vars: &Vars, s: &String) -> io::Result<String> {
    let mut vars = vars.clone();
    for i in 0..vars.len() {
        vars[i].1 = eval_with_finalized_vars(&vars[..i], &vars[i].1)?;
    }
    eval_with_finalized_vars(&vars[..], s)
}

fn eval_with_finalized_vars(vars: &[(String, String)], s: &String) -> io::Result<String> {
    let env_keys: Vec<String> = std::env::vars().map(|(key, _)| key).collect();

    let mut eval = process::Command::new("bash");
    let mut cwd = std::env::current_dir()?;
    for (key, value) in vars {
        // If the key is in the environment, we should ignore it.
        // This allows users to override with the environment variables in CLI.
        if env_keys.contains(key) {
            continue;
        }
        eval.env(key, value);
        if key == "OSDK_CWD" {
            cwd = std::path::PathBuf::from(value);
        }
    }
    eval.arg("-c");
    eval.arg(format!("echo \"{}\"", s));
    eval.current_dir(cwd);
    let output = eval.output()?;
    if !output.stderr.is_empty() {
        println!(
            "[Info] {}",
            String::from_utf8_lossy(&output.stderr).trim_end_matches('\n')
        );
    }
    Ok(String::from_utf8_lossy(&output.stdout)
        .trim_end_matches('\n')
        .to_string())
}
