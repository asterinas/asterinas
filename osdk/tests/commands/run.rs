// SPDX-License-Identifier: MPL-2.0

//! Test the `run` command

const WORKSPACE: &str = "/tmp/kernel_test_workspace/run_command";

mod workspace {
    use crate::util::*;
    use std::{
        fs::{create_dir_all, remove_dir_all},
        path::Path,
    };

    fn create_kernel_in_workspace(workspace: &str, kernel_name: &str) {
        let os_dir = Path::new(workspace).join(kernel_name);
        if os_dir.exists() {
            remove_dir_all(&os_dir).unwrap();
        }
        let mut cargo_osdk_new = cargo_osdk(["new", "--kernel", kernel_name]);
        cargo_osdk_new.current_dir(workspace);
        cargo_osdk_new
            .ok()
            .expect("Failed to create kernel project");
    }

    fn prepare_workspace(workspace: &str) {
        if !Path::new(workspace).exists() {
            create_dir_all(workspace).unwrap();
        }
    }
    #[derive(Debug)]
    pub struct WorkSpace {
        pub workspace: String,
        pub kernel_name: String,
    }
    impl WorkSpace {
        pub fn new(workspace: &str, kernel_name: &str) -> Self {
            prepare_workspace(workspace);
            create_kernel_in_workspace(workspace, kernel_name);
            Self {
                workspace: workspace.to_string(),
                kernel_name: kernel_name.to_string(),
            }
        }

        pub fn os_dir(&self) -> String {
            Path::new(&self.workspace)
                .join(self.kernel_name.clone())
                .to_string_lossy()
                .to_string()
        }
    }
    impl Drop for WorkSpace {
        fn drop(&mut self) {
            remove_dir_all(&self.os_dir()).unwrap();
        }
    }
}

mod qemu_gdb_feature {
    use super::*;
    use crate::util::cargo_osdk;
    use assert_cmd::Command;
    use std::{path::Path, thread::sleep};

    #[test]
    fn basic_debug() {
        let workspace = workspace::WorkSpace::new(WORKSPACE, "basic_debug");
        let unix_socket = {
            let path = Path::new(&workspace.os_dir()).join("qemu-gdb-sock");
            path.to_string_lossy().to_string()
        };

        let mut instance = cargo_osdk(["run", "-G", "--gdb-server-addr", unix_socket.as_str()]);
        instance.current_dir(&workspace.os_dir());

        let sock = unix_socket.clone();
        let _gdb = std::thread::spawn(move || {
            gdb_continue_via_unix_sock(&sock);
        });

        let output = instance
            .output()
            .expect("Failed to wait for QEMU GDB instance");
        let stdout = String::from_utf8_lossy(&output.stdout).to_string();
        assert!(stdout.contains("Hello world from guest kernel!"));
    }

    fn gdb_continue_via_unix_sock(socket: &str) {
        let sock_file = Path::new(&socket);
        while !sock_file.exists() {
            sleep(std::time::Duration::from_secs(1));
        }
        gdb_continue_via(socket);
    }

    fn gdb_continue_via(addr: &str) {
        let mut gdb = Command::new("gdb");
        gdb.args(["-ex", format!("target remote {}", addr).as_str()]);
        gdb.write_stdin("\n")
            .write_stdin("c\n")
            .write_stdin("quit\n");
        gdb.assert().success();
    }
    mod vsc {
        use super::*;

        #[test]
        fn vsc_launch_file() {
            let kernel_name = "vsc_launch_file";
            let workspace = workspace::WorkSpace::new(WORKSPACE, kernel_name);
            let addr = ":50001";

            let mut instance = cargo_osdk(["run", "-G", "--vsc", "--gdb-server-addr", addr]);
            instance.current_dir(&workspace.os_dir());

            let dir = workspace.os_dir();
            let bin_file_path = Path::new(&workspace.os_dir())
                .join("target")
                .join("x86_64-unknown-none")
                .join("debug")
                .join(format!("{}-osdk-bin", kernel_name));
            let _gdb = std::thread::spawn(move || {
                while !bin_file_path.exists() {
                    sleep(std::time::Duration::from_secs(1));
                }
                assert!(
                    check_launch_file_existence(&dir),
                    "VSCode launch config file is not found during debugging session"
                );
                gdb_continue_via(&addr);
            });

            let output = instance
                .output()
                .expect("Failed to wait for QEMU GDB instance");
            let stdout = String::from_utf8_lossy(&output.stdout).to_string();
            assert!(stdout.contains("Hello world from guest kernel!"));
            assert!(
                !check_launch_file_existence(&workspace.workspace),
                "VSCode launch config file should be removed after debugging session"
            );
        }

        fn check_launch_file_existence(workspace: &str) -> bool {
            let launch_file = Path::new(workspace).join(".vscode/launch.json");
            launch_file.exists()
        }
    }
}
