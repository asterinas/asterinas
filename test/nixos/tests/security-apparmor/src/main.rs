// SPDX-License-Identifier: MPL-2.0

//! End-to-end interoperability with the official AppArmor parser.

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn apparmor_parser_upstream_regressions(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd_and_expect_regex(
        r#"(
            aa_tests=$(mktemp -d) &&
            trap 'rm -rf "$aa_tests"' EXIT &&
            cp -R /etc/apparmor-parser-tests/. "$aa_tests/" &&
            cd "$aa_tests" &&
            APPARMOR_PARSER=$(command -v apparmor_parser) prove ./simple.pl &&
            APPARMOR_PARSER="$(command -v apparmor_parser) --config-file=./parser.conf" sh ./dirtest.sh
        )"#,
        &Regex::new("All tests successful").unwrap(),
    )?;
    Ok(())
}

#[nixos_test]
fn apparmor_parser_compile_load_and_query(nixos_shell: &mut Session) -> Result<(), Error> {
    let success = Regex::new("(?s).*").unwrap();
    nixos_shell.run_cmd_and_expect_regex(
        "apparmor_parser --version",
        &Regex::new(r"AppArmor parser version 5\.0\.2").unwrap(),
    )?;
    nixos_shell.run_cmd_and_expect_regex(
        "test \"$(cat /sys/kernel/security/apparmor/features/policy/versions/v5)\" = yes",
        &success,
    )?;
    nixos_shell.run_cmd_and_expect_regex(
        r#"(
            aa_dir=$(mktemp -d) &&
            trap 'rm -rf "$aa_dir"' EXIT &&
            apparmor_parser --stdout /etc/apparmor-parser-tests/dirtest/gooddir/profileA > "$aa_dir/a.bin" &&
            apparmor_parser --Complain --stdout /etc/apparmor-parser-tests/dirtest/gooddir/profileB > "$aa_dir/b.bin" &&
            apparmor_parser --binary --add "$aa_dir/b.bin" &&
            apparmor_parser --binary --add "$aa_dir/a.bin" &&
            test "$(cat /sys/kernel/security/apparmor/profiles)" = "$(printf 'a_profile (enforce)\nb_profile (complain)\n')" &&
            aa_snapshot=$(cat /sys/kernel/security/apparmor/profiles) &&
            if LC_ALL=C apparmor_parser --binary --add "$aa_dir/a.bin" > "$aa_dir/duplicate.log" 2>&1; then
                false
            else
                grep -F 'File exists' "$aa_dir/duplicate.log" &&
                test "$(cat /sys/kernel/security/apparmor/profiles)" = "$aa_snapshot"
            fi
        )"#,
        &success,
    )?;
    Ok(())
}
