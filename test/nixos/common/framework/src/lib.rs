// SPDX-License-Identifier: MPL-2.0

//! An imperative testing framework for NixOS-based tests.
//!
//! # Core Concepts
//!
//! ## Test Registration
//!
//! Use the `#[nixos_test]` attribute to register test cases. The framework
//! automatically discovers and runs all registered tests.
//!
//! ## Session Interaction
//!
//! Tests are implemented by interacting with a [`Session`] object. The [`Session`] type
//! provides methods for executing commands and verifying output. It supports nested execution
//! contexts (containers, SSH, etc.) with automatic cleanup.
//!
//! See the [template crate](https://github.com/asterinas/asterinas/tree/main/test/nixos/common/template)
//! for a usage example.
//!
//! See the [project README](https://github.com/asterinas/asterinas/tree/main/test/nixos)
//! for complete documentation on creating and running test suites.

use std::env;

pub use inventory;
pub use nixos_test_macro::nixos_test;
pub use rexpect::error::Error;
pub use session::{Session, SessionDesc};

mod session;

/// A test case definition.
pub struct TestCase {
    pub name: &'static str,
    pub test_fn: fn(&mut Session) -> Result<(), Error>,
}

inventory::collect!(TestCase);

/// Generates the main function that runs all test cases.
#[macro_export]
macro_rules! nixos_test_main {
    () => {
        fn main() -> Result<(), Box<dyn std::error::Error>> {
            $crate::__nixos_test_main()
        }
    };
}

#[doc(hidden)]
pub fn __nixos_test_main() -> Result<(), Box<dyn std::error::Error>> {
    let args: Vec<String> = env::args().collect();

    // Check for --help flag
    if args.contains(&"--help".to_string()) || args.contains(&"-h".to_string()) {
        print_help();
        return Ok(());
    }

    // Parse --qemu-cmd argument
    let qemu_cmd = parse_arg(&args, "--qemu-cmd").ok_or("Missing --qemu-cmd argument")?;

    // Parse optional --test argument
    let test_filter = parse_arg(&args, "--test");

    // Parse timeout from NIXOS_TEST_TIMEOUT environment variable
    let timeout_ms = env::var("NIXOS_TEST_TIMEOUT")
        .ok()
        .map(|v| parse_timeout(&v))
        .transpose()?
        .unwrap_or(300_000); // Default: 5 minutes

    let all_test_cases: Vec<&TestCase> = inventory::iter::<TestCase>().collect();

    if all_test_cases.is_empty() {
        return Err("No test cases found".into());
    }

    // Filter test cases if --test is specified
    let test_cases: Vec<&TestCase> = if let Some(ref filter) = test_filter {
        let filtered: Vec<&TestCase> = all_test_cases
            .into_iter()
            .filter(|tc| tc.name == filter)
            .collect();

        if filtered.is_empty() {
            return Err(format!("Test case '{}' not found", filter).into());
        }

        filtered
    } else {
        all_test_cases
    };

    if let Some(ref filter) = test_filter {
        println!("=== Running single test case: {} ===", filter);
    } else {
        println!("=== Found {} test case(s) ===", test_cases.len());
        for tc in &test_cases {
            println!("  - test_{}", tc.name);
        }
        println!();
    }

    let mut session = rexpect::spawn(&qemu_cmd, Some(timeout_ms))?;

    println!("--> Waiting for login prompt...");
    let init_prompt = "root@asterinas:";
    session.exp_string(init_prompt)?;

    let desc = SessionDesc::new()
        .expect_prompt(init_prompt)
        .cmd_to_enter("")
        .cmd_to_exit("poweroff");
    let mut session = Session::new(desc, session);

    let mut passed = 0;
    let mut failed = 0;
    let mut failed_tests = Vec::new();

    for test_case in test_cases {
        println!("=== Running test case: {} ===", test_case.name);

        match session.run(test_case.test_fn) {
            Ok(_) => {
                println!("✓ Test case 'test_{}' passed\n", test_case.name);
                passed += 1;
            }
            Err(_) => {
                println!("✗ Test case 'test_{}' failed\n", test_case.name);
                failed += 1;
                failed_tests.push(test_case.name);
            }
        }
    }

    println!("=== Test Summary ===");
    println!("Passed: {}", passed);
    println!("Failed: {}", failed);

    let res = if !failed_tests.is_empty() {
        println!("\nFailed tests:");
        for name in failed_tests {
            println!("  - test_{}", name);
        }

        Err("Some tests failed")
    } else {
        Ok(())
    };

    let shutdown_res = session.shutdown();

    res?;
    shutdown_res?;

    Ok(())
}

/// Parses timeout string with units into milliseconds.
///
/// Supports formats:
/// - `<number>ms` - milliseconds
/// - `<number>s` - seconds
/// - `<number>min` - minutes
///
/// # Examples
///
/// ```rust
/// parse_timeout("300000ms") // Ok(300000)
/// parse_timeout("300s")     // Ok(300000)
/// parse_timeout("5min")     // Ok(300000)
/// ```
fn parse_timeout(timeout_str: &str) -> Result<u64, Box<dyn std::error::Error>> {
    let timeout_str = timeout_str.trim();

    if let Some(ms) = timeout_str.strip_suffix("ms") {
        return Ok(ms.trim().parse()?);
    }

    if let Some(s) = timeout_str.strip_suffix('s') {
        let seconds: u64 = s.trim().parse()?;
        return Ok(seconds * 1000);
    }

    if let Some(m) = timeout_str.strip_suffix("min") {
        let minutes: u64 = m.trim().parse()?;
        return Ok(minutes * 60000);
    }

    Err(format!(
        "Invalid timeout format '{}'. Use: <number>ms, <number>s, or <number>m",
        timeout_str
    )
    .into())
}

/// Parse command line argument in the form --flag <value>
fn parse_arg(args: &[String], flag: &str) -> Option<String> {
    for i in 0..args.len() {
        if args[i] == flag {
            return args.get(i + 1).cloned();
        }
    }
    None
}

fn print_help() {
    println!(
        "\
NixOS-Based Test Framework

USAGE:
    <test-binary> --qemu-cmd <COMMAND> [OPTIONS]

REQUIRED ARGUMENTS:
    --qemu-cmd <COMMAND>    Command to launch QEMU with the test environment

OPTIONS:
    --test <TEST_NAME>      Run only the specified test case
    -h, --help              Print this help message

ENVIRONMENT VARIABLES:
    NIXOS_TEST_TIMEOUT      Timeout for command execution
                            Supports: <number>ms, <number>s, <number>min
                            Examples: 300000ms, 300s, 5min
                            (default: 5min = 300000ms)

"
    );
}
