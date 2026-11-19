// SPDX-License-Identifier: MPL-2.0

use std::{
    fs,
    io::{BufRead, BufReader},
    process::{Command, Stdio},
    sync::{Arc, Mutex},
    thread,
    time::Duration,
};

use assert_cmd::{cargo::CommandCargoExt, output::OutputOkExt};

use crate::util::{cargo_osdk, edit_config_files};

#[test]
fn write_a_hypervisor_in_100_lines_riscv() {
    let workdir = tempfile::tempdir().unwrap();
    let os_name = "hypervisor_in_100_lines_riscv";
    let os_dir = workdir.path().join(os_name);

    cargo_osdk(["new", "--kernel", os_name])
        .current_dir(workdir.path())
        .ok()
        .unwrap();

    for (path, contents) in [
        (
            "src/lib.rs",
            include_str!("write_a_hypervisor_in_100_lines_riscv_templates/lib.rs"),
        ),
        (
            "OSDK.toml",
            include_str!("write_a_hypervisor_in_100_lines_riscv_templates/OSDK.toml"),
        ),
        (
            "Makefile",
            include_str!("write_a_hypervisor_in_100_lines_riscv_templates/Makefile"),
        ),
        (
            "guest.S",
            include_str!("write_a_hypervisor_in_100_lines_riscv_templates/guest.S"),
        ),
    ] {
        fs::write(os_dir.join(path), contents).unwrap();
    }
    edit_config_files(&os_dir);

    let output = Command::new("make")
        .arg("guest")
        .current_dir(&os_dir)
        .ok()
        .unwrap();
    assert!(
        output.status.success(),
        "Failed to build the guest image:\n{}",
        String::from_utf8_lossy(&output.stderr)
    );
    // Unlike the x86 demo, the RISC-V hypervisor is always run under QEMU's
    // software emulation, so the target architecture must be selected
    // explicitly instead of following the host architecture.
    let mut command = Command::cargo_bin("cargo-osdk").unwrap();
    command
        .arg("osdk")
        .args(["run", "--target-arch", "riscv64"])
        .current_dir(&os_dir)
        .stdout(Stdio::piped());
    let mut child = command.spawn().unwrap();

    // QEMU forwards the guest's serial console to stdout. Print it line by
    // line while the test runs so that it is visible on the screen, and keep
    // a copy for the assertion below.
    let stdout = child.stdout.take().unwrap();

    // Terminate QEMU if the guest does not shut it down in time.
    let child = Arc::new(Mutex::new(child));
    let _watchdog = {
        let child = Arc::clone(&child);
        thread::spawn(move || {
            thread::sleep(Duration::from_secs(300));
            let _ = child.lock().unwrap().kill();
        })
    };

    let mut output = String::new();
    for line in BufReader::new(stdout).lines() {
        let line = line.unwrap();
        println!("{line}");
        output.push_str(&line);
        output.push('\n');
    }

    let status = child.lock().unwrap().wait().unwrap();
    assert!(status.success(), "`cargo osdk run` failed:\n{output}");
    assert!(output.contains("Hello World"), "{output}");
}
