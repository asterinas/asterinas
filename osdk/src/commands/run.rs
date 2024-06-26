// SPDX-License-Identifier: MPL-2.0

use super::{build::create_base_and_cached_build, util::DEFAULT_TARGET_RELPATH};
use crate::{
    cli::GdbServerArgs,
    config::{scheme::ActionChoice, unix_args::split_to_kv_array, Config},
    util::{get_current_crate_info, get_target_directory},
};

pub fn execute_run_command(config: &Config, gdb_server_args: &GdbServerArgs) {
    if gdb_server_args.is_gdb_enabled {
        use std::env;
        env::set_var(
            "RUSTFLAGS",
            env::var("RUSTFLAGS").unwrap_or_default() + " -g",
        );
    }

    let cargo_target_directory = get_target_directory();
    let osdk_output_directory = cargo_target_directory.join(DEFAULT_TARGET_RELPATH);
    let target_name = get_current_crate_info().name;

    let mut config = config.clone();
    if gdb_server_args.is_gdb_enabled {
        let qemu_gdb_args = {
            let gdb_stub_addr = gdb_server_args.gdb_server_addr.as_str();
            match gdb::stub_type_of(gdb_stub_addr) {
                gdb::StubAddrType::Unix => {
                    format!(
                        " -chardev socket,path={},server=on,wait=off,id=gdb0 -gdb chardev:gdb0 -S",
                        gdb_stub_addr
                    )
                }
                gdb::StubAddrType::Tcp => {
                    format!(
                        " -gdb tcp:{} -S",
                        gdb::tcp_addr_util::format_tcp_addr(gdb_stub_addr)
                    )
                }
            }
        };
        config.run.qemu.args += &qemu_gdb_args;

        // FIXME: Disable KVM from QEMU args in debug mode.
        // Currently, the QEMU GDB server does not work properly with KVM enabled.
        let mut splitted = split_to_kv_array(&config.run.qemu.args);
        let args_num = splitted.len();
        splitted.retain(|x| !x.contains("kvm"));
        if splitted.len() != args_num {
            println!(
                "[WARNING] KVM is forced to be disabled in GDB server currently. \
                    Options related with KVM are ignored."
            );
        }

        config.run.qemu.args = splitted.join(" ");

        // Ensure debug info added when debugging in the release profile.
        if config.run.build.profile.contains("release") {
            config
                .run
                .build
                .override_configs
                .push(format!("profile.{}.debug=true", config.run.build.profile));
        }
    }
    let _vsc_launch_file = gdb_server_args.vsc_launch_file.then(|| {
        vsc::check_gdb_config(gdb_server_args);
        let profile = super::util::profile_name_adapter(&config.run.build.profile);
        vsc::VscLaunchConfig::new(profile, &gdb_server_args.gdb_server_addr)
    });

    let default_bundle_directory = osdk_output_directory.join(target_name);
    let bundle = create_base_and_cached_build(
        default_bundle_directory,
        &osdk_output_directory,
        &cargo_target_directory,
        &config,
        ActionChoice::Run,
        &[],
    );

    bundle.run(&config, ActionChoice::Run);
}

mod gdb {
    #[derive(Debug, Clone, Copy, PartialEq, Eq)]
    pub enum StubAddrType {
        Unix, // Unix Domain Socket
        Tcp,  // IP_ADDR:PORT
    }
    pub fn stub_type_of(stub: &str) -> StubAddrType {
        if stub.split(':').last().unwrap().parse::<u16>().is_ok() {
            return StubAddrType::Tcp;
        }
        StubAddrType::Unix
    }

    pub mod tcp_addr_util {
        use crate::{error::Errno, error_msg};
        use std::process::exit;

        fn strip_tcp_prefix(addr: &str) -> &str {
            addr.strip_prefix("tcp:").unwrap_or(addr)
        }

        fn parse_tcp_addr(addr: &str) -> (&str, u16) {
            let addr = strip_tcp_prefix(addr);
            if !addr.contains(':') {
                error_msg!("Ambiguous GDB server address, use '[IP]:PORT' format");
                exit(Errno::ParseMetadata as _);
            }
            let mut iter = addr.split(':');
            let host = iter.next().unwrap();
            let port = iter.next().unwrap().parse().unwrap();
            (host, port)
        }

        pub fn format_tcp_addr(tcp_addr: &str) -> String {
            let (host, port) = parse_tcp_addr(tcp_addr);
            format!("{}:{}", host, port)
        }
    }
}

mod vsc {
    use crate::{
        cli::GdbServerArgs,
        commands::util::bin_file_name,
        util::{get_cargo_metadata, get_current_crate_info},
    };
    use serde_json::{from_str, Value};
    use std::{
        fs::{read_to_string, write as write_file},
        path::Path,
    };

    use super::gdb;

    const VSC_DIR: &str = ".vscode";

    #[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
    struct Existence {
        vsc_dir: bool,
        launch_file: bool,
    }

    fn workspace_root() -> String {
        get_cargo_metadata(None::<&str>, None::<&[&str]>).unwrap()["workspace_root"]
            .as_str()
            .unwrap()
            .to_owned()
    }

    #[derive(Debug, Default)]
    pub struct VscLaunchConfig {
        existence: Existence,
        backup_launch_path: Option<String>,
    }
    impl VscLaunchConfig {
        pub fn new(profile: &str, addr: &str) -> Self {
            let workspace = workspace_root();
            let workspace = Path::new(&workspace);
            let launch_file_path = workspace.join(VSC_DIR).join("launch.json");
            let existence = Existence {
                vsc_dir: workspace.join(VSC_DIR).exists(),
                launch_file: launch_file_path.exists(),
            };

            let backup_launch_path = existence.launch_file.then(|| {
                let backup_launch_path = launch_file_path.with_extension("bak");
                std::fs::copy(&launch_file_path, &backup_launch_path).unwrap();
                backup_launch_path.to_string_lossy().to_string()
            });

            if !existence.vsc_dir {
                std::fs::create_dir(workspace.join(VSC_DIR)).unwrap();
            }
            generate_vsc_launch_file(workspace, profile, addr).unwrap();

            VscLaunchConfig {
                existence,
                backup_launch_path,
            }
        }
    }
    impl Drop for VscLaunchConfig {
        fn drop(&mut self) {
            // remove generated files
            if !self.existence.vsc_dir {
                std::fs::remove_dir_all(Path::new(&workspace_root()).join(VSC_DIR)).unwrap();
                return;
            }
            if !self.existence.launch_file {
                std::fs::remove_file(
                    Path::new(&workspace_root())
                        .join(VSC_DIR)
                        .join("launch.json"),
                )
                .unwrap();
                return;
            }
            // restore backup launch file
            if let Some(backup_launch_path) = &self.backup_launch_path {
                std::fs::copy(
                    backup_launch_path,
                    Path::new(&workspace_root())
                        .join(VSC_DIR)
                        .join("launch.json"),
                )
                .unwrap();
                std::fs::remove_file(backup_launch_path).unwrap();
            }
        }
    }

    /// Exit if the QEMU GDB server configuration is not valid
    pub fn check_gdb_config(args: &GdbServerArgs) {
        use crate::{error::Errno, error_msg};
        use std::process::exit;

        if !args.is_gdb_enabled {
            error_msg!(
                "No need for a VSCode launch file without launching GDB server,\
                    pass '-h' for help"
            );
            exit(Errno::ParseMetadata as _);
        }

        // check GDB server address
        let gdb_stub_addr = args.gdb_server_addr.as_str();
        if gdb_stub_addr.is_empty() {
            error_msg!("GDB server address is required to generate a VSCode launch file");
            exit(Errno::ParseMetadata as _);
        }
        if gdb::stub_type_of(gdb_stub_addr) != gdb::StubAddrType::Tcp {
            error_msg!("Non-TCP GDB server address is not supported under '--vsc' currently");
            exit(Errno::ParseMetadata as _);
        }
    }

    fn generate_vsc_launch_file(
        workspace: impl AsRef<Path>,
        profile: &str,
        addr: &str,
    ) -> Result<(), std::io::Error> {
        let contents = include_str!("launch.json.template")
            .replace("#PROFILE#", profile)
            .replace("#CRATE_NAME#", &get_current_crate_info().name)
            .replace("#BIN_NAME#", &bin_file_name())
            .replace(
                "#ADDR_PORT#",
                gdb::tcp_addr_util::format_tcp_addr(addr).trim_start_matches(':'),
            );

        let original_items: Option<Value> = {
            let launch_file_path = workspace.as_ref().join(VSC_DIR).join("launch.json");
            let src_path = if launch_file_path.exists() {
                launch_file_path
            } else {
                launch_file_path.with_extension("bak")
            };
            src_path
                .exists()
                .then(|| from_str(&read_to_string(&src_path).unwrap()).unwrap())
        };

        let contents = if let Some(mut original_items) = original_items {
            let items: Value = from_str(&contents)?;
            for item in items["configurations"].as_array().unwrap() {
                if original_items["configurations"]
                    .as_array()
                    .unwrap()
                    .iter()
                    .any(|x| x["name"].as_str() == item["name"].as_str())
                {
                    println!("[{}/launch.json]{} already exists", VSC_DIR, item["name"]);
                    // no override for configurations with the same name
                    continue;
                }
                original_items["configurations"]
                    .as_array_mut()
                    .unwrap()
                    .push(item.clone());
            }
            serde_json::to_string_pretty(&original_items).unwrap()
        } else {
            contents
        };

        let launch_file_path = workspace.as_ref().join(VSC_DIR).join("launch.json");
        write_file(launch_file_path, contents)
    }
}
