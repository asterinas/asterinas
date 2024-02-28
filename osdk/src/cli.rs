// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::{crate_version, Args, Parser};

use crate::{
    commands::{
        execute_build_command, execute_check_command, execute_clippy_command, execute_new_command,
        execute_run_command, execute_test_command,
    },
    config_manager::{
        boot::{BootLoader, BootProtocol},
        qemu::QemuMachine,
        BuildConfig, RunConfig, TestConfig,
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
        OsdkSubcommand::Test(test_args) => {
            let test_config = TestConfig::parse(test_args);
            execute_test_command(&test_config);
        }
        OsdkSubcommand::Check => execute_check_command(),
        OsdkSubcommand::Clippy => execute_clippy_command(),
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
    #[command(about = "Execute kernel mode unit test by starting a VMM")]
    Test(TestArgs),
    #[command(about = "Analyze the current package and report errors")]
    Check,
    #[command(about = "Check the current package and catch common mistakes")]
    Clippy,
}

#[derive(Debug, Parser)]
pub struct NewArgs {
    #[arg(long, default_value = "false", help = "Use the kernel template")]
    pub kernel: bool,
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
        default_value = "dev"
    )]
    pub profile: String,
    #[arg(long, value_name = "FEATURES", help = "List of features to activate")]
    pub features: Vec<String>,
}

#[derive(Debug, Args)]
pub struct OsdkArgs {
    #[arg(
        long = "select",
        help = "Select the specific configuration provided in the OSDK manifest",
        value_name = "SELECTION"
    )]
    pub select: Option<String>,
    #[arg(
        long = "kcmd_args",
        help = "Command line arguments for guest kernel",
        value_name = "ARGS"
    )]
    pub kcmd_args: Vec<String>,
    #[arg(
        long = "init_args",
        help = "Command line arguments for init process",
        value_name = "ARGS"
    )]
    pub init_args: Vec<String>,
    #[arg(long, help = "Path of initramfs", value_name = "PATH")]
    pub initramfs: Option<PathBuf>,
    #[arg(long = "boot.ovmf", help = "Path of OVMF", value_name = "PATH")]
    pub boot_ovmf: Option<PathBuf>,
    #[arg(
        long = "boot.loader",
        help = "Loader for booting the kernel",
        value_name = "LOADER"
    )]
    pub boot_loader: Option<BootLoader>,
    #[arg(
        long = "boot.grub-mkrescue",
        help = "Path of grub-mkrescue",
        value_name = "PATH"
    )]
    pub boot_grub_mkrescue: Option<PathBuf>,
    #[arg(
        long = "boot.protocol",
        help = "Protocol for booting the kernel",
        value_name = "PROTOCOL"
    )]
    pub boot_protocol: Option<BootProtocol>,
    #[arg(long = "qemu.path", help = "Path of QEMU", value_name = "PATH")]
    pub qemu_path: Option<PathBuf>,
    #[arg(
        long = "qemu.machine",
        help = "QEMU machine type",
        value_name = "MACHINE"
    )]
    pub qemu_machine: Option<QemuMachine>,
    #[arg(
        long = "qemu.args",
        help = "Arguments for running QEMU",
        value_name = "ARGS"
    )]
    pub qemu_args: Vec<String>,
}
