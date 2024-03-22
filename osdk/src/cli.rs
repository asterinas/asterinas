// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::{crate_version, Args, Parser};

use crate::{
    arch::Arch,
    commands::{
        execute_build_command, execute_debug_command, execute_forwarded_command,
        execute_new_command, execute_run_command, execute_test_command,
    },
    config_manager::{
        action::{BootProtocol, Bootloader},
        manifest::ProjectType,
        BuildConfig, DebugConfig, RunConfig, TestConfig,
    },
};

pub fn main() {
    let osdk_subcommand = match Cli::parse() {
        Cli {
            cargo_subcommand: CargoSubcommand::Osdk(osdk_subcommand),
        } => osdk_subcommand,
    };

    match &osdk_subcommand {
        OsdkSubcommand::New(args) => execute_new_command(args),
        OsdkSubcommand::Build(build_args) => {
            let build_config = BuildConfig::parse(build_args);
            execute_build_command(&build_config);
        }
        OsdkSubcommand::Run(run_args) => {
            let run_config = RunConfig::parse(run_args);
            execute_run_command(&run_config);
        }
        OsdkSubcommand::Debug(debug_args) => {
            let debug_config = DebugConfig::parse(debug_args);
            execute_debug_command(&debug_config);
        }
        OsdkSubcommand::Test(test_args) => {
            let test_config = TestConfig::parse(test_args);
            execute_test_command(&test_config);
        }
        OsdkSubcommand::Check(args) => execute_forwarded_command("check", &args.args),
        OsdkSubcommand::Clippy(args) => execute_forwarded_command("clippy", &args.args),
        OsdkSubcommand::Doc(args) => execute_forwarded_command("doc", &args.args),
    }
}

#[derive(Debug, Parser)]
#[clap(display_name = "cargo", bin_name = "cargo")]
/// Project Manager for the crates developed based on frame kernel
pub struct Cli {
    #[clap(subcommand)]
    cargo_subcommand: CargoSubcommand,
}

#[derive(Debug, Parser)]
enum CargoSubcommand {
    #[clap(subcommand, version = crate_version!())]
    Osdk(OsdkSubcommand),
}

#[derive(Debug, Parser)]
pub enum OsdkSubcommand {
    #[command(
        about = "Create a new kernel package or library package which depends on aster-frame"
    )]
    New(NewArgs),
    #[command(about = "Compile the project and its dependencies")]
    Build(BuildArgs),
    #[command(about = "Run the kernel with a VMM")]
    Run(RunArgs),
    #[command(about = "Debug a remote target via GDB")]
    Debug(DebugArgs),
    #[command(about = "Execute kernel mode unit test by starting a VMM")]
    Test(TestArgs),
    #[command(about = "Check a local package and all of its dependencies for errors")]
    Check(ForwardedArguments),
    #[command(about = "Checks a package to catch common mistakes and improve your Rust code")]
    Clippy(ForwardedArguments),
    #[command(about = "Build a package's documentation")]
    Doc(ForwardedArguments),
}

#[derive(Debug, Parser)]
pub struct ForwardedArguments {
    #[arg(
        help = "The full set of Cargo arguments",
        trailing_var_arg = true,
        allow_hyphen_values = true
    )]
    pub args: Vec<String>,
}

#[derive(Debug, Parser)]
pub struct NewArgs {
    #[arg(
        long = "type",
        short = 't',
        default_value = "library",
        help = "The type of the project to create"
    )]
    pub type_: ProjectType,
    #[arg(name = "name", required = true)]
    pub crate_name: String,
}

#[derive(Debug, Parser)]
pub struct BuildArgs {
    #[command(flatten)]
    pub cargo_args: CargoArgs,
    #[command(flatten)]
    pub osdk_args: OsdkArgs,
}

#[derive(Debug, Parser)]
pub struct RunArgs {
    #[command(flatten)]
    pub cargo_args: CargoArgs,
    #[command(flatten)]
    pub osdk_args: OsdkArgs,
    #[command(flatten)]
    pub gdb_server_args: GdbServerArgs,
}

#[derive(Debug, Args, Clone, Default)]
pub struct GdbServerArgs {
    /// Whether to enable QEMU GDB server for debugging
    #[arg(
        long = "enable-gdb",
        short = 'G',
        help = "Enable QEMU GDB server for debugging",
        default_value_t
    )]
    pub is_gdb_enabled: bool,
    #[arg(
        long = "vsc",
        help = "Generate a '.vscode/launch.json' for debugging with Visual Studio Code \
                (only works when '--enable-gdb' is enabled)",
        default_value_t
    )]
    pub vsc_launch_file: bool,
    #[arg(
        long = "gdb-server-addr",
        help = "The network address on which the GDB server listens, \
        it can be either a path for the UNIX domain socket or a TCP port on an IP address.",
        value_name = "ADDR",
        default_value = ".aster-gdb-socket"
    )]
    pub gdb_server_addr: String,
}

#[derive(Debug, Parser)]
pub struct DebugArgs {
    #[command(flatten)]
    pub cargo_args: CargoArgs,
    #[command(flatten)]
    pub osdk_args: OsdkArgs,
    #[arg(
        long,
        help = "Specify the address of the remote target",
        default_value = ".aster-gdb-socket"
    )]
    pub remote: String,
}

#[derive(Debug, Parser)]
pub struct TestArgs {
    #[command(flatten)]
    pub cargo_args: CargoArgs,
    #[arg(
        name = "TESTNAME",
        help = "Only run tests containing this string in their names"
    )]
    pub test_name: Option<String>,
    #[command(flatten)]
    pub osdk_args: OsdkArgs,
}

#[derive(Debug, Args, Default, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CargoArgs {
    #[arg(
        long,
        help = "The Cargo build profile (built-in candidates are 'dev', 'release', 'test' and 'bench')",
        default_value = "dev",
        conflicts_with = "release"
    )]
    pub profile: String,
    #[arg(
        long,
        help = "Build artifacts in release mode",
        conflicts_with = "profile"
    )]
    pub release: bool,
    #[arg(long, value_name = "FEATURES", help = "List of features to activate", value_delimiter = ',', num_args = 1..)]
    pub features: Vec<String>,
}

#[derive(Debug, Args)]
pub struct OsdkArgs {
    #[arg(long, value_name = "ARCH", help = "The architecture to build for")]
    pub arch: Option<Arch>,
    #[arg(
        long = "schema",
        help = "Select the specific configuration schema provided in the OSDK manifest",
        value_name = "SCHEMA"
    )]
    pub schema: Option<String>,
    #[arg(
        long = "kcmd_args+",
        require_equals = true,
        help = "Extra or overriding command line arguments for guest kernel",
        value_name = "ARGS"
    )]
    pub kcmd_args: Vec<String>,
    #[arg(
        long = "init_args+",
        require_equals = true,
        help = "Extra command line arguments for init process",
        value_name = "ARGS"
    )]
    pub init_args: Vec<String>,
    #[arg(long, help = "Path of initramfs", value_name = "PATH")]
    pub initramfs: Option<PathBuf>,
    #[arg(long = "ovmf", help = "Path of OVMF", value_name = "PATH")]
    pub ovmf: Option<PathBuf>,
    #[arg(long = "opensbi", help = "Path of OpenSBI", value_name = "PATH")]
    pub opensbi: Option<PathBuf>,
    #[arg(
        long = "bootloader",
        help = "Loader for booting the kernel",
        value_name = "BOOTLOADER"
    )]
    pub bootloader: Option<Bootloader>,
    #[arg(
        long = "grub-mkrescue",
        help = "Path of grub-mkrescue",
        value_name = "PATH"
    )]
    pub grub_mkrescue: Option<PathBuf>,
    #[arg(
        long = "boot_protocol",
        help = "Protocol for booting the kernel",
        value_name = "BOOT_PROTOCOL"
    )]
    pub boot_protocol: Option<BootProtocol>,
    #[arg(
        long = "qemu_exe",
        help = "The QEMU executable file",
        value_name = "FILE"
    )]
    pub qemu_exe: Option<PathBuf>,
    #[arg(
        long = "qemu_args+",
        require_equals = true,
        help = "Extra arguments or overriding arguments for running QEMU",
        value_name = "ARGS"
    )]
    pub qemu_args_add: Vec<String>,
}
