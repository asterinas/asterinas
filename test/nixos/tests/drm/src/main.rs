// SPDX-License-Identifier: MPL-2.0

//! Smoke tests for the DRM, Xorg, and Mesa integration path on Asterinas NixOS.

use std::time::Duration;

use nixos_test_framework::*;

nixos_test_main!();

#[nixos_test]
fn drm_xorg_mesa_smoke(nixos_shell: &mut Session) -> Result<(), Error> {
    // Wait for Xorg and GLX, retaining the output for the checks below.
    nixos_shell.wait_until_check_matches(
        &CommandCheck::new(
            "bash -o pipefail -c 'timeout 2s env DISPLAY=:0 glxinfo -B 2>&1 | tee /tmp/drm-glxinfo.txt'",
            "",
        ),
        Duration::from_secs(90),
        "Xorg and GLX to become ready",
    )?;

    // An empty pattern checks only the command's exit status.
    nixos_shell.run_cmd_and_expect_regex("test -c /dev/dri/card0", &Regex::new("").unwrap())?;
    nixos_shell.run_cmd_and_expect_regex(
        "grep -Fx 'direct rendering: Yes' /tmp/drm-glxinfo.txt",
        &Regex::new("direct rendering: Yes").unwrap(),
    )?;
    nixos_shell.run_cmd_and_expect_regex(
        "grep -F 'OpenGL vendor string:' /tmp/drm-glxinfo.txt | grep -F Mesa",
        &Regex::new("Mesa").unwrap(),
    )?;
    nixos_shell.run_cmd_and_expect_regex(
        "DISPLAY=:0 xrandr --query",
        &Regex::new(r"(?m)^[^[:space:]]+ connected( primary)? [0-9]+x[0-9]+\+[0-9]+\+[0-9]+")
            .unwrap(),
    )?;

    // Verify that Xorg uses the DRM device even when Mesa renders in software.
    nixos_shell.run_cmd_and_expect_regex(
        r#"readlink /proc/"$(pgrep -o -x Xorg)"/fd/* 2>/dev/null | grep -Fx /dev/dri/card0"#,
        &Regex::new("/dev/dri/card0").unwrap(),
    )?;

    Ok(())
}
