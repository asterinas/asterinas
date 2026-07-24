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

    // On NixOS, the Python interpreter's stdlib lives on the read-only `/nix/store`
    // bind mount. Some `test.support` helpers `unlink()` `.pyc` files across every
    // `sys.path` entry, which fails with `EROFS` there. The stdlib `sys.path`
    // entries are derived from `sys.prefix`, so copying that prefix onto writable
    // tmpfs moves them off the read-only mount.
    nixos_shell.run_cmd("mkdir -p /tmp/pyhome")?;
    nixos_shell.run_cmd("export PREFIX=$(python3 -c 'import sys; print(sys.prefix)')")?;
    nixos_shell.run_cmd("cp -dR --preserve=mode $PREFIX/lib /tmp/pyhome/lib")?;
    nixos_shell.run_cmd("cp -dR --preserve=mode $PREFIX/include /tmp/pyhome/include")?;
    nixos_shell.run_cmd("rm -rf /tmp/pyhome/lib/python3.12/test")?;
    nixos_shell.run_cmd("cp -a /tmp/python3-src/Lib/test /tmp/pyhome/lib/python3.12")?;
    nixos_shell.run_cmd("export PYTHONHOME=/tmp/pyhome")?;

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
        let cmd = format!("python3 -m test -u all {testcase}");
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
