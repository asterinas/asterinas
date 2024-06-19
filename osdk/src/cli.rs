// SPDX-License-Identifier: MPL-2.0

use std::path::PathBuf;

use clap::{crate_version, Args, Parser};

use crate::{
    arch::Arch,
    commands::{
        execute_build_command, execute_debug_command, execute_forwarded_command,
        execute_new_command, execute_run_command, execute_test_command,
    },
    config::{
        manifest::{ProjectType, TomlManifest},
        scheme::{BootMethod, BootProtocol},
        Config,
    },
};

pub fn main() {
    let load_config = |common_args: &CommonArgs| {
        let manifest = TomlManifest::load();
        let scheme = manifest.get_scheme(common_args.scheme.as_ref());
        Config::new(scheme, common_args)
    };

    let cli = Cli::parse();
    let CargoSubcommand::Osdk(osdk_subcommand) = &cli.cargo_subcommand;

    match osdk_subcommand {
        OsdkSubcommand::New(args) => execute_new_command(args),
        OsdkSubcommand::Build(build_args) => {
            execute_build_command(&load_config(&build_args.common_args), build_args);
        }
        OsdkSubcommand::Run(run_args) => {
            execute_run_command(
                &load_config(&run_args.common_args),
                &run_args.gdb_server_args,
            );
        }
        OsdkSubcommand::Debug(debug_args) => {
            execute_debug_command(
                &load_config(&debug_args.common_args).run.build.profile,
                debug_args,
            );
        }
        OsdkSubcommand::Test(test_args) => {
            execute_test_command(&load_config(&test_args.common_args), test_args);
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
    #[command(about = "Create a new kernel package or library package which depends on OSTD")]
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
        id = "type",
        long = "type",
        short = 't',
        default_value = "library",
        help = "The type of the project to create",
        conflicts_with_all = ["kernel", "library"],
    )]
    pub type_: ProjectType,
    #[arg(
        long,
        help = "Create a kernel package",
        conflicts_with_all = ["library", "type"],
    )]
    pub kernel: bool,
    #[arg(
        long,
        alias = "lib",
        help = "Create a library package",
        conflicts_with_all = ["kernel", "type"],
    )]
    pub library: bool,
    #[arg(name = "name", required = true)]
    pub crate_name: String,
}

impl NewArgs {
    pub fn project_type(&self) -> ProjectType {
        if self.kernel {
            ProjectType::Kernel
        } else if self.library {
            ProjectType::Library
        } else {
            self.type_
        }
    }
}

#[derive(Debug, Parser)]
pub struct BuildArgs {
    #[arg(
        long = "for-test",
        help = "Build for running unit tests",
        default_value_t
    )]
    pub for_test: bool,
    #[arg(
        long = "output",
        short = 'o',
        help = "Output directory for all generated artifacts",
        value_name = "DIR"
    )]
    pub output: Option<PathBuf>,
    #[command(flatten)]
    pub common_args: CommonArgs,
}

#[derive(Debug, Parser)]
pub struct RunArgs {
    #[command(flatten)]
    pub gdb_server_args: GdbServerArgs,
    #[command(flatten)]
    pub common_args: CommonArgs,
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
    #[arg(
        long,
        help = "Specify the address of the remote target",
        default_value = ".aster-gdb-socket"
    )]
    pub remote: String,
    #[command(flatten)]
    pub common_args: CommonArgs,
}

#[derive(Debug, Parser)]
pub struct TestArgs {
    #[arg(
        name = "TESTNAME",
        help = "Only run tests containing this string in their names"
    )]
    pub test_name: Option<String>,
    #[command(flatten)]
    pub common_args: CommonArgs,
}

#[derive(Debug, Args, Default, Clone, Eq, PartialEq, Serialize, Deserialize)]
pub struct CargoArgs {
    #[arg(
        long,
        help = "The Cargo build profile (built-in candidates are 'dev', 'release', 'test' and 'bench')",
        conflicts_with = "release",
        global = true
    )]
    pub profile: Option<String>,
    #[arg(
        long,
        help = "Build artifacts in release mode",
        conflicts_with = "profile",
        global = true
    )]
    pub release: bool,
    #[arg(
        long,
        value_name = "FEATURES",
        help = "List of features to activate",
        value_delimiter = ',',
        num_args = 1..,
        global = true,
    )]
    pub features: Vec<String>,
    #[arg(long, help = "Do not activate the `default` features", global = true)]
    pub no_default_features: bool,
    #[arg(
        long = "config",
        help = "Override a configuration value",
        value_name = "KEY=VALUE",
        global = true
    )]
    pub override_configs: Vec<String>,
}

impl CargoArgs {
    pub fn profile(&self) -> Option<String> {
        if self.release {
            Some("release".to_owned())
        } else {
            self.profile.clone()
        }
    }
}

#[derive(Debug, Args)]
/// Common args used for build, run, test and debug subcommand
pub struct CommonArgs {
    #[command(flatten)]
    pub build_args: CargoArgs,
    #[arg(
        long = "linux-x86-legacy-boot",
        help = "Enable legacy 32-bit boot support for the Linux x86 boot protocol",
        global = true
    )]
    pub linux_x86_legacy_boot: bool,
    #[arg(
        long = "strip-elf",
        help = "Strip the built kernel ELF file for a smaller size",
        global = true
    )]
    pub strip_elf: bool,
    #[arg(
        long = "target-arch",
        value_name = "ARCH",
        help = "The architecture to build for",
        global = true
    )]
    pub target_arch: Option<Arch>,
    #[arg(
        long = "scheme",
        help = "Select the specific configuration scheme provided in the OSDK manifest",
        value_name = "SCHEME",
        global = true
    )]
    pub scheme: Option<String>,
    #[arg(
        long = "kcmd-args",
        require_equals = true,
        help = "Extra or overriding command line arguments for guest kernel",
        value_name = "ARGS",
        global = true
    )]
    pub kcmd_args: Vec<String>,
    #[arg(
        long = "init-args",
        require_equals = true,
        help = "Extra command line arguments for init process",
        value_name = "ARGS",
        global = true
    )]
    pub init_args: Vec<String>,
    #[arg(long, help = "Path of initramfs", value_name = "PATH", global = true)]
    pub initramfs: Option<PathBuf>,
    #[arg(
        long = "boot-method",
        help = "Loader for booting the kernel",
        value_name = "BOOTMETHOD",
        global = true
    )]
    pub boot_method: Option<BootMethod>,
    #[arg(
        long = "bootdev-append-options",
        help = "Additional QEMU `-drive` options for the boot device",
        value_name = "OPTIONS",
        global = true
    )]
    pub bootdev_append_options: Option<String>,
    #[arg(
        long = "display-grub-menu",
        help = "Display the GRUB menu if booting with GRUB",
        global = true
    )]
    pub display_grub_menu: bool,
    #[arg(
        long = "grub-mkrescue",
        help = "Path of grub-mkrescue",
        value_name = "PATH",
        global = true
    )]
    pub grub_mkrescue: Option<PathBuf>,
    #[arg(
        long = "grub-boot-protocol",
        help = "Protocol for booting the kernel",
        value_name = "BOOT_PROTOCOL",
        global = true
    )]
    pub grub_boot_protocol: Option<BootProtocol>,
    #[arg(
        long = "qemu-exe",
        help = "The QEMU executable file",
        value_name = "FILE",
        global = true
    )]
    pub qemu_exe: Option<PathBuf>,
    #[arg(
        long = "qemu-args",
        require_equals = true,
        help = "Extra arguments or overriding arguments for running QEMU",
        value_name = "ARGS",
        global = true
    )]
    pub qemu_args: Vec<String>,
}
