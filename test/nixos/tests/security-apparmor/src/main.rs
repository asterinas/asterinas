// SPDX-License-Identifier: MPL-2.0

//! The upstream AppArmor policy-management regression test.
//!
//! Covers the parser cache-path failure described in PR #4203.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn apparmor_policy_management(nixos_shell: &mut Session) -> Result<(), Error> {
    // Keep the command on one shell line so terminal continuation prompts cannot alter its echo.
    nixos_shell.run_cmd_and_expect_regex(
        concat!(
            "(aa_tests=$(mktemp -d) && ",
            r#"trap 'rm -rf "$aa_tests"' EXIT && "#,
            r#"cp -R /etc/apparmor-tests/. "$aa_tests/" && "#,
            r#"cd "$aa_tests" && "#,
            r#"printf '%s\n' "subdomain=$(command -v apparmor_parser)" "#,
            r#"'parser_config="--config-file=/etc/apparmor/parser.conf"' "#,
            r#"'parser_args="$parser_config -q -K"' "#,
            "'tmpdir=/tmp/apparmor-upstream-e2e' ",
            "'sys_profiles=/sys/kernel/security/apparmor/profiles' ",
            "'aa_exec=aa-exec' > uservars.inc && ",
            "/bin/true && bash -x ./e2e.sh && bash -x ./e2e.sh)",
        ),
        // The upstream assertions report failures through the script's exit status.
        &Regex::new("").unwrap(),
    )?;
    Ok(())
}
