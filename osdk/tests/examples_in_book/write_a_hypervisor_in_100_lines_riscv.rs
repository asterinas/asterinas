// SPDX-License-Identifier: MPL-2.0

use std::{fs, path::Path, process::Command};

use assert_cmd::output::OutputOkExt;

use crate::util;

#[test]
fn build_riscv_hypervisor_in_100_lines() {
    let workdir = Path::new("/tmp");
    let os_name = "riscv_hypervisor_in_100_lines";
    let os_dir = workdir.join(os_name);

    if !os_dir.exists() {
        util::cargo_osdk(["new", "--kernel", os_name])
            .current_dir(workdir)
            .ok()
            .unwrap();
    }
    util::depends_on_local_ostd(os_dir.join("Cargo.toml"));

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

    Command::new("make")
        .arg("guest")
        .current_dir(&os_dir)
        .ok()
        .unwrap();
    let guest_size = fs::metadata(os_dir.join("guest")).unwrap().len();
    // The example loads the entire guest image into one RISC-V base page.
    const GUEST_PAGE_SIZE: u64 = 4096;
    assert!(
        guest_size > 0 && guest_size <= GUEST_PAGE_SIZE,
        "guest image must fit in one page, got {guest_size} bytes"
    );

    util::cargo_osdk(["build", "--target-arch", "riscv64"])
        .current_dir(&os_dir)
        .ok()
        .unwrap();
}
