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

#[nixos_test]
fn text_attachment_exec_and_external_pid_query(nixos_shell: &mut Session) -> Result<(), Error> {
    // Shell builtins read the helper's own attribute, then signal readiness.
    // No child `cat` process stands in for the target's self-query.
    // Quote the shebang literally: this shell has interactive history expansion.
    nixos_shell.run_cmd_and_expect(
        r#"mkdir -p /tmp/aa-e2e && { printf '#!%s\n' "$(command -v bash)"; printf '%s\n' 'IFS= read -r label < /proc/self/attr/current' 'printf "%s\n" "$label" > "$1"' 'IFS= read -r release < "$2"'; } > /tmp/aa-e2e/a && chmod +x /tmp/aa-e2e/a && cp /tmp/aa-e2e/a /tmp/aa-e2e/b && cp /tmp/aa-e2e/a /tmp/aa-e2e/u && echo helpers-ready"#,
        "helpers-ready",
    )?;
    nixos_shell.run_cmd(
        r#"printf 'profile e2e-a /tmp/aa-e2e/a {\n}\n' > /tmp/aa-e2e/a.profile && printf 'profile e2e-b /tmp/aa-e2e/b flags=(complain) {\n}\n' > /tmp/aa-e2e/b.profile"#,
    )?;
    nixos_shell.run_cmd_and_expect(
        "apparmor_parser --stdout /tmp/aa-e2e/a.profile > /tmp/aa-e2e/a.bin && apparmor_parser --binary --add /tmp/aa-e2e/a.bin && apparmor_parser --stdout /tmp/aa-e2e/b.profile > /tmp/aa-e2e/b.bin && apparmor_parser --binary --add /tmp/aa-e2e/b.bin && echo attachments-loaded",
        "attachments-loaded",
    )?;
    nixos_shell.run_cmd(
        "mkfifo /tmp/aa-e2e/ready-a /tmp/aa-e2e/gate-a /tmp/aa-e2e/ready-b /tmp/aa-e2e/gate-b /tmp/aa-e2e/ready-u /tmp/aa-e2e/gate-u; /tmp/aa-e2e/a /tmp/aa-e2e/ready-a /tmp/aa-e2e/gate-a & aa_pid_a=$!; /tmp/aa-e2e/b /tmp/aa-e2e/ready-b /tmp/aa-e2e/gate-b & aa_pid_b=$!; /tmp/aa-e2e/u /tmp/aa-e2e/ready-u /tmp/aa-e2e/gate-u & aa_pid_u=$!",
    )?;
    nixos_shell.run_cmd_and_expect(
        r#"IFS= read -r aa_self_a < /tmp/aa-e2e/ready-a; IFS= read -r aa_self_b < /tmp/aa-e2e/ready-b; IFS= read -r aa_self_u < /tmp/aa-e2e/ready-u; printf 'labels: <%s>|<%s>|<%s>\n' "$aa_self_a" "$aa_self_b" "$aa_self_u"; test "$aa_self_a" = 'e2e-a (enforce)' && test "$aa_self_b" = 'e2e-b (complain)' && test "$aa_self_u" = unconfined && echo self-queries-correct"#,
        "self-queries-correct",
    )?;
    nixos_shell.run_cmd_and_expect(
        r#"test "$(cat /proc/$aa_pid_a/attr/current)" = "$aa_self_a" && test "$(cat /proc/$aa_pid_b/attr/current)" = "$aa_self_b" && test "$(cat /proc/$aa_pid_u/attr/current)" = "$aa_self_u" && echo external-pid-queries-correct"#,
        "external-pid-queries-correct",
    )?;
    nixos_shell.run_cmd(
        "echo go > /tmp/aa-e2e/gate-a; echo go > /tmp/aa-e2e/gate-b; echo go > /tmp/aa-e2e/gate-u; wait $aa_pid_a $aa_pid_b $aa_pid_u; rm -r /tmp/aa-e2e",
    )?;
    Ok(())
}
