// SPDX-License-Identifier: MPL-2.0

//! The module implementing the evaluation feature.

use std::{io, path::Path, process};

/// This function is used to evaluate the string using the host's shell recursively
/// in order.
pub fn eval(cwd: impl AsRef<Path>, s: &String) -> io::Result<String> {
    let mut eval = process::Command::new("bash");
    eval.arg("-c");
    eval.arg(format!("echo \"{}\"", s));
    eval.current_dir(cwd.as_ref());
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
