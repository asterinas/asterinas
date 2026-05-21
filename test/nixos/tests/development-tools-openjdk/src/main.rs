// SPDX-License-Identifier: MPL-2.0

//! The test suite for OpenJDK on Asterinas NixOS.
//!
//! Supplements the `OpenJDK` coverage in `development-tools`.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn openjdk_regression(nixos_shell: &mut Session) -> Result<(), Error> {
    // `jtreg -v1 -nr <path>` ends each invocation with a `Test results:` summary
    // line. A fully successful run uses the exact form:
    //
    //   Test results: passed: <count>
    //
    // When any selected test fails, errors out, or is skipped, the summary is
    // no longer this success-only form and instead includes additional result
    // categories. Each invocation here runs `jtreg` for exactly one entry from
    // `src/testcases.txt`, so matching the exact success summary is sufficient.
    let success_regex =
        Regex::new(r"(?m)^Test results: passed: [0-9]+$").map_err(|err| Error::Pty(err.into()))?;

    let testcases =
        std::fs::read_to_string("src/testcases.txt").map_err(|err| Error::Pty(err.into()))?;
    let mut failed_tests = Vec::new();
    for testcase in testcases
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        let cmd = format!("jtreg -v1 -nr -retain:none /tmp/jdk-src/test/jdk/{testcase}");
        if let Err(err) = nixos_shell.run_cmd_and_expect_regex(&cmd, &success_regex) {
            failed_tests.push((testcase.to_string(), err));
        }
    }

    if !failed_tests.is_empty() {
        println!("=== Failed OpenJDK regression tests ===");
        for (test, _) in &failed_tests {
            println!("  - {test}");
        }
        println!("==============================");

        return Err(Error::Aggregated {
            summary: "Selected OpenJDK regression tests failed!".to_string(),
            collections: failed_tests,
        });
    }

    Ok(())
}
