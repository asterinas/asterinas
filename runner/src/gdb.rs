// SPDX-License-Identifier: MPL-2.0

//! Providing the utility to run the GDB scripts for the runner.

use crate::qemu_grub_efi;
use std::{fs::OpenOptions, io::Write, path::PathBuf, process::Command};

/// Run a GDB client.
///
/// If argument `gdb_grub` is set true, it will run GRUB's gdb script.
///
/// Make sure to set GRUB_PREFIX to the actual GRUB you are using.
///
/// When debugging grub, the OVMF firmware will load the grub kernel at an
/// address unknown at the moment. You should use the debug message from our
/// custom built OVMF firmware and read the entrypoint address
/// (often `0x0007E684000`). Then use the following GDB command to load symbols:
/// `dynamic_load_symbols ${ENTRY_ADDRESS}`.
/// During each run, the address is unlikely to change. But the address will
/// depend on the versions of grub or OVMF.
///
/// Also, do `set breakpoint pending on` when you want to break on GRUB modules.
pub fn run_gdb_client(path: &PathBuf, gdb_grub: bool) {
    let path = std::fs::canonicalize(path).unwrap();
    let mut gdb_cmd = Command::new("gdb");
    // Set the architecture, otherwise GDB will complain about.
    gdb_cmd.arg("-ex").arg("set arch i386:x86-64:intel");
    let grub_script = "/tmp/aster-gdb-grub-script";
    if gdb_grub {
        let grub_dir = PathBuf::from(qemu_grub_efi::GRUB_PREFIX)
            .join("lib")
            .join("grub")
            .join(qemu_grub_efi::GRUB_VERSION);
        // Load symbols from GRUB using the provided grub gdb script.
        // Read the contents from `gdb_grub` and
        // replace the lines containing "target remote :1234".
        gdb_cmd.current_dir(&grub_dir);
        let grub_script_content = std::fs::read_to_string(grub_dir.join("gdb_grub")).unwrap();
        let lines = grub_script_content.lines().collect::<Vec<_>>();
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .open(grub_script)
            .unwrap();
        for line in lines {
            if line.contains("target remote :1234") {
                // Connect to the GDB server.
                writeln!(f, "target remote /tmp/aster-gdb-socket").unwrap();
            } else {
                writeln!(f, "{}", line).unwrap();
            }
        }
        gdb_cmd.arg("-x").arg(grub_script);
    } else {
        // Load symbols from the kernel image.
        gdb_cmd.arg("-ex").arg(format!("file {}", path.display()));
        // Connect to the GDB server.
        gdb_cmd
            .arg("-ex")
            .arg("target remote /tmp/aster-gdb-socket");
    }
    // Connect to the GDB server and run.
    println!("running:{:#?}", gdb_cmd);
    gdb_cmd.status().unwrap();
    if gdb_grub {
        // Clean the temporary script file then return.
        std::fs::remove_file(grub_script).unwrap();
    }
}
