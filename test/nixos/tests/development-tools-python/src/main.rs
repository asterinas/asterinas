// SPDX-License-Identifier: MPL-2.0

//! The test suite for Python 3 on Asterinas NixOS.
//!
//! Supplements the `Python 3` coverage in `development-tools`.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn python3_regrtest(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir /tmp/python3-src")?;
    nixos_shell
        .run_cmd("tar xf /tmp/python3-src.tar.xz --strip-components=1 -C /tmp/python3-src")?;
    // Regrtest removes legacy .pyc paths under every sys.path entry. Relocate
    // the installed standard library out of the read-only /nix/store mount.
    nixos_shell.run_cmd(
        "python_root=$(dirname $(dirname $(readlink -f $(command -v python3)))); mkdir /tmp/python3-home; cp -a $python_root/lib /tmp/python3-home/lib; ln -s $python_root/include /tmp/python3-home/include",
    )?;
    const RUN_REGRTEST: &str = concat!(
        "import os, runpy, sys; sys.path.insert(0, '/tmp/python3-src/Lib'); ",
        "os.environ.pop('PYTHONPATH', None); ",
        "sys.argv = ['test', '-u', 'all', *sys.argv[1:]]; ",
        "runpy.run_module('test', run_name='__main__')",
    );

    // Printed once by regrtest at the end of every run as `Result: {state}`:
    //   https://github.com/python/cpython/blob/v3.12.12/Lib/test/libregrtest/main.py#L455-L456
    // `state` is the literal "SUCCESS" iff no failure/env-change/interrupt/etc.
    // flag is set; any failure produces a disjoint string ("FAILURE",
    // "NO TESTS RAN, INTERRUPTED", ...), so this substring match is sufficient:
    //   https://github.com/python/cpython/blob/v3.12.12/Lib/test/libregrtest/results.py#L55-L69
    const RESULT_SUCCESS: &str = "Result: SUCCESS";

    let testcases =
        std::fs::read_to_string("src/testcases.txt").map_err(|err| Error::Pty(err.into()))?;
    let mut failed_tests = Vec::new();
    for testcase in testcases
        .lines()
        .map(str::trim)
        .filter(|l| !l.is_empty() && !l.starts_with('#'))
    {
        // test_cmd_line validates the behavior of child interpreters with
        // PYTHONPATH unset, so add the source path to this process only.
        let cmd = if testcase.split_whitespace().next() == Some("test_cmd_line") {
            format!("PYTHONHOME=/tmp/python3-home python3 -c \"{RUN_REGRTEST}\" {testcase}")
        } else {
            format!(
                "PYTHONHOME=/tmp/python3-home PYTHONPATH=/tmp/python3-src/Lib python3 -m test -u all {testcase}"
            )
        };
        if let Err(err) = nixos_shell.run_cmd_and_expect(&cmd, RESULT_SUCCESS) {
            failed_tests.push((testcase.to_string(), err));
        }
    }

    if !failed_tests.is_empty() {
        println!("=== Failed Python 3 regression tests ===");
        for (test, _) in &failed_tests {
            println!("  - {test}");
        }
        println!("======================================");

        return Err(Error::Aggregated {
            summary: "Selected Python 3 regression tests failed!".to_string(),
            collections: failed_tests,
        });
    }

    Ok(())
}
