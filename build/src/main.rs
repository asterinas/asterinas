//! jinux-runner is the Jinux runner script to ease the pain of running
//! and testing Jinux inside a QEMU VM. It should be built and run as the
//! cargo runner: https://doc.rust-lang.org/cargo/reference/config.html
//!
//! The runner will generate the filesystem image for starting Jinux. If
//! we should use the runner in the default mode, which invokes QEMU with
//! a GRUB boot device image, the runner would be responsible for generating
//! the and the boot device image. It also supports directly boot the
//! kernel image without GRUB using the QEMU microvm mode.
//!

pub mod machine;

use std::{
    fs::OpenOptions,
    io::Write,
    path::{Path, PathBuf},
    process::Command,
};

use clap::Parser;

use crate::machine::{default, microvm};

/// The CLI of this runner.
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    // Positional arguments.
    /// The Jinux binary path.
    path: PathBuf,

    /// Provide the kernel commandline, which specifies
    /// the init process.
    kcmdline: String,

    // Optional arguments.
    /// Boot method. Can be one of the following items:
    ///  - `grub-multiboot`,
    ///  - `grub-multiboot2`,
    ///  - `grub-linux`,
    ///  - `microvm-multiboot`.
    #[arg(long, default_value = "grub-multiboot2")]
    boot_method: String,

    /// Enable KVM when running QEMU.
    #[arg(long, default_value_t = false)]
    enable_kvm: bool,

    /// Emulate Intel IOMMU by QEMU.
    #[arg(long, default_value_t = false)]
    emulate_iommu: bool,

    /// Run QEMU as a GDB server.
    #[arg(long, default_value_t = false)]
    halt_for_gdb: bool,

    /// Boot without displaying the GRUB menu.
    #[arg(long, default_value_t = false)]
    skip_grub_menu: bool,

    /// Run a GDB client instead of running the kernel.
    #[arg(long, default_value_t = false)]
    run_gdb_client: bool,
}

pub const COMMON_ARGS: &[&str] = &[
    "--no-reboot",
    "-cpu",
    "Icelake-Server,+x2apic",
    "-m",
    "2G",
    "-nographic", // TODO: figure out why grub can't shown up without it
    "-monitor",
    "vc",
    "-serial",
    "mon:stdio",
    "-display",
    "none",
    "-device",
    "isa-debug-exit,iobase=0xf4,iosize=0x04",
    "-object",
    "filter-dump,id=filter0,netdev=net01,file=virtio-net.pcap",
];

pub fn random_hostfwd_netdev_arg() -> String {
    let start = 32768u16;
    let end = 61000u16;
    let port1 = rand::random::<u16>() % (end - 1 - start) + start;
    let port2 = rand::random::<u16>() % (end - port1) + port1;
    format!(
        "user,id=net01,hostfwd=tcp::{}-:22,hostfwd=tcp::{}-:8080",
        port1, port2
    )
}

pub const GDB_ARGS: &[&str] = &[
    "-chardev",
    "socket,path=/tmp/jinux-gdb-socket,server=on,wait=off,id=gdb0",
    "-gdb",
    "chardev:gdb0",
    "-S",
];

fn run_gdb_client(path: &PathBuf, gdb_grub: bool) {
    let path = std::fs::canonicalize(path).unwrap();
    let mut gdb_cmd = Command::new("gdb");
    // Set the architecture, otherwise GDB will complain about.
    gdb_cmd.arg("-ex").arg("set arch i386:x86-64:intel");
    let grub_script = "/tmp/jinux-gdb-grub-script";
    if gdb_grub {
        // Load symbols from GRUB using the provided grub gdb script.
        // Read the contents from /usr/lib/grub/i386-pc/gdb_grub and
        // replace the lines containing "file kernel.exec" and
        // "target remote :1234".
        gdb_cmd.current_dir("/usr/lib/grub/i386-pc/");
        let grub_script_content = include_str!("/usr/lib/grub/i386-pc/gdb_grub");
        let lines = grub_script_content.lines().collect::<Vec<_>>();
        let mut f = OpenOptions::new()
            .write(true)
            .create(true)
            .open(grub_script)
            .unwrap();
        for line in lines {
            if line.contains("target remote :1234") {
                // Connect to the GDB server.
                writeln!(f, "target remote /tmp/jinux-gdb-socket").unwrap();
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
            .arg("target remote /tmp/jinux-gdb-socket");
    }
    // Connect to the GDB server and run.
    println!("running:{:#?}", gdb_cmd);
    gdb_cmd.status().unwrap();
    if gdb_grub {
        // Clean the temporary script file then return.
        std::fs::remove_file(grub_script).unwrap();
    }
}

fn main() {
    let args = Args::parse();

    if args.run_gdb_client {
        let gdb_grub = args.boot_method.contains("grub");
        // You should comment out this code if you want to debug gdb instead
        // of the kernel because this argument is not exposed by runner CLI.
        // let gdb_grub = gdb_grub && false;
        run_gdb_client(&args.path, gdb_grub);
        return;
    }

    let mut qemu_cmd = Command::new("qemu-system-x86_64");

    qemu_cmd.args(COMMON_ARGS);

    qemu_cmd.arg("-netdev");
    qemu_cmd.arg(random_hostfwd_netdev_arg().as_str());

    if args.halt_for_gdb {
        if args.enable_kvm {
            println!("Runner: Can't enable KVM when running QEMU as a GDB server. Abort.");
            return;
        }
        qemu_cmd.args(GDB_ARGS);
    }

    if args.enable_kvm {
        qemu_cmd.arg("-enable-kvm");
    }
    // Specify machine type
    if args.boot_method == "microvm-multiboot" {
        qemu_cmd.args(microvm::MACHINE_ARGS);
    } else {
        qemu_cmd.args(default::MACHINE_ARGS);
    }
    // Add device arguments
    if args.boot_method == "microvm-multiboot" {
        qemu_cmd.args(microvm::DEVICE_ARGS);
    } else if args.emulate_iommu {
        qemu_cmd.args(default::IOMMU_DEVICE_ARGS);
    } else {
        qemu_cmd.args(default::NOIOMMU_DEVICE_ARGS);
    }

    let fs_image = create_fs_image(args.path.as_path());
    qemu_cmd.arg("-drive");
    qemu_cmd.arg(fs_image);

    if args.boot_method == "microvm-multiboot" {
        let image = microvm::create_bootdev_image(args.path);
        qemu_cmd.arg("-kernel");
        qemu_cmd.arg(image.as_os_str());
        qemu_cmd.arg("-append");
        qemu_cmd.arg(&args.kcmdline);
        qemu_cmd.arg("-initrd");
        qemu_cmd.arg("regression/build/initramfs.cpio.gz");
    } else {
        let boot_protocol = match args.boot_method.as_str() {
            "grub-multiboot" => default::GrubBootProtocol::Multiboot,
            "grub-multiboot2" => default::GrubBootProtocol::Multiboot2,
            "grub-linux" => default::GrubBootProtocol::Linux,
            _ => panic!("Unknown boot method: {}", args.boot_method),
        };
        let grub_cfg = default::generate_grub_cfg(
            "build/grub/grub.cfg.template",
            &args.kcmdline,
            args.skip_grub_menu,
            boot_protocol,
        );
        let bootdev_image = default::create_bootdev_image(args.path, grub_cfg, boot_protocol);
        qemu_cmd.arg("-cdrom");
        qemu_cmd.arg(bootdev_image.as_os_str());
    }

    println!("running:{:#?}", qemu_cmd);

    let exit_status = qemu_cmd.status().unwrap();
    if !exit_status.success() {
        // FIXME: Exit code manipulation is not needed when using non-x86 QEMU
        let qemu_exit_code = exit_status.code().unwrap();
        let kernel_exit_code = qemu_exit_code >> 1;
        match kernel_exit_code {
            0x10 /*jinux_frame::QemuExitCode::Success*/ => { std::process::exit(0); },
            0x20 /*jinux_frame::QemuExitCode::Failed*/ => { std::process::exit(1); },
            _ => { std::process::exit(qemu_exit_code) },
        }
    }
}

pub fn create_fs_image(path: &Path) -> String {
    let mut fs_img_path = path.parent().unwrap().to_str().unwrap().to_string();
    fs_img_path.push_str("/fs.img");
    let path = Path::new(fs_img_path.as_str());
    if path.exists() {
        return format!("file={},if=none,format=raw,id=x0", fs_img_path.as_str());
    }
    let f = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .open(fs_img_path.as_str())
        .unwrap();
    // 32MiB
    f.set_len(64 * 1024 * 1024).unwrap();
    format!("file={},if=none,format=raw,id=x0", fs_img_path.as_str())
}
