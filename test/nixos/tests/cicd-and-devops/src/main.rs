// SPDX-License-Identifier: MPL-2.0

//! The test suite for CI/CD and DevOps applications on Asterinas NixOS.
//!
//! See `test/nixos/README.md#documentation-maintenance` for sync requirements
//! between this test suite and the corresponding "Verified Usage" book section.

use nixos_test_framework::*;

nixos_test_main!();

// ============================================================================
// CI/CD Runners - just
// ============================================================================

#[nixos_test]
fn just_run_recipe(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/just-test")?;
    nixos_shell
        .run_cmd(r#"cd /tmp/just-test && echo -e 'build:\n\techo "Hello from Just"' > justfile"#)?;

    nixos_shell.run_cmd_and_expect("cd /tmp/just-test && just --list", "build")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/just-test && just build", "Hello from Just")?;
    Ok(())
}

// ============================================================================
// CI/CD Runners - Task
// ============================================================================

#[nixos_test]
fn task_run_task(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/task-test")?;
    nixos_shell.run_cmd(
        r#"cd /tmp/task-test && echo -e 'version: 3\ntasks:\n  build:\n    cmds:\n      - echo "Hello from Task"' > taskfile.yml"#,
    )?;

    nixos_shell.run_cmd_and_expect("cd /tmp/task-test && task --list-all", "build")?;
    nixos_shell.run_cmd_and_expect("cd /tmp/task-test && task build", "Hello from Task")?;
    Ok(())
}

// ============================================================================
// Release Automation - GoReleaser
// ============================================================================

#[nixos_test]
fn goreleaser_release_project(nixos_shell: &mut Session) -> Result<(), Error> {
    nixos_shell.run_cmd("mkdir -p /tmp/goreleaser-test")?;
    nixos_shell.run_cmd("cd /tmp/goreleaser-test && go mod init goreleaser-test")?;
    nixos_shell.run_cmd(r#"cd /tmp/goreleaser-test && echo -e 'package main\nimport "fmt"\nfunc main() { fmt.Println("Hello from GoReleaser") }' > main.go"#)?;

    nixos_shell.run_cmd_and_expect("cd /tmp/goreleaser-test && goreleaser init", "done")?;
    nixos_shell.run_cmd_and_expect(
        "cd /tmp/goreleaser-test && goreleaser release --snapshot --clean",
        "succeeded",
    )?;

    nixos_shell.run_cmd_and_expect(
        "ls /tmp/goreleaser-test/dist/goreleaser-test_linux_amd64*",
        "goreleaser-test",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ls /tmp/goreleaser-test/dist/goreleaser-test_linux_arm64*",
        "goreleaser-test",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ls /tmp/goreleaser-test/dist/goreleaser-test_windows_amd64*",
        "goreleaser-test.exe",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ls /tmp/goreleaser-test/dist/goreleaser-test_windows_arm64*",
        "goreleaser-test.exe",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ls /tmp/goreleaser-test/dist/goreleaser-test_darwin_amd64*",
        "goreleaser-test",
    )?;
    nixos_shell.run_cmd_and_expect(
        "ls /tmp/goreleaser-test/dist/goreleaser-test_darwin_arm64*",
        "goreleaser-test",
    )?;

    nixos_shell.run_cmd_and_expect(
        "find /tmp/goreleaser-test/dist/ -name 'goreleaser-test' -path '*linux_amd64*' -executable -exec {} \\;",
        "Hello from GoReleaser",
    )?;
    Ok(())
}
