// SPDX-License-Identifier: MPL-2.0

use std::{fs, process::Command, time::Duration};

use assert_cmd::output::OutputOkExt;

use crate::util::{cargo_osdk, depends_on_local_ostd};

#[ignore = "requires nested VMX"]
#[test]
fn write_a_hypervisor_in_100_lines() {
    let workdir = tempfile::tempdir().unwrap();
    let os_name = "hypervisor_in_100_lines";
    let os_dir = workdir.path().join(os_name);

    cargo_osdk(["new", "--kernel", os_name])
        .current_dir(workdir.path())
        .ok()
        .unwrap();
    depends_on_local_ostd(os_dir.join("Cargo.toml"));

    for (path, contents) in [
        (
            "src/lib.rs",
            include_str!("write_a_hypervisor_in_100_lines_templates/lib.rs"),
        ),
        (
            "OSDK.toml",
            include_str!("write_a_hypervisor_in_100_lines_templates/OSDK.toml"),
        ),
        (
            "Makefile",
            include_str!("write_a_hypervisor_in_100_lines_templates/Makefile"),
        ),
        (
            "guest_hello.S",
            include_str!("write_a_hypervisor_in_100_lines_templates/guest_hello.S"),
        ),
    ] {
        fs::write(os_dir.join(path), contents).unwrap();
    }

    Command::new("make")
        .arg("guest_hello")
        .current_dir(&os_dir)
        .ok()
        .unwrap();

    let output = cargo_osdk(["run"])
        .current_dir(&os_dir)
        .timeout(Duration::from_secs(300))
        .ok()
        .unwrap();
    let stdout = String::from_utf8_lossy(&output.stdout);
    assert!(stdout.contains("Hello World"), "{stdout}");
    assert!(
        stdout.contains("Guest halted; returned to host."),
        "{stdout}"
    );
}
