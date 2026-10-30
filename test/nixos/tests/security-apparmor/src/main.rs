// SPDX-License-Identifier: MPL-2.0

//! End-to-end interoperability with the official AppArmor parser.

use nixos_test_framework::*;

nixos_test_main!();

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
        r#"aa_dir=$(mktemp -d) && trap 'rm -rf "$aa_dir"' EXIT && printf 'profile observe flags=(complain) {\n}\n' > "$aa_dir/profile""#,
        &success,
    )?;
    nixos_shell.run_cmd_and_expect_regex(
        r#"apparmor_parser --stdout "$aa_dir/profile" > "$aa_dir/policy.bin" && apparmor_parser --binary --add "$aa_dir/policy.bin" && test "$(cat /sys/kernel/security/apparmor/profiles)" = 'observe (complain)' && aa_snapshot=$(cat /sys/kernel/security/apparmor/profiles)"#,
        &success,
    )?;
    nixos_shell.run_cmd_and_expect_regex(
        r#"if apparmor_parser --binary --add "$aa_dir/policy.bin" > "$aa_dir/duplicate.log" 2>&1; then false; else grep -F 'File exists' "$aa_dir/duplicate.log" && test "$(cat /sys/kernel/security/apparmor/profiles)" = "$aa_snapshot"; fi"#,
        &success,
    )?;
    nixos_shell.run_cmd_and_expect_regex(r#"rm -r "$aa_dir" && trap - EXIT"#, &success)?;
    Ok(())
}
